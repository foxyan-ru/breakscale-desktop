//! Terraform export: hand-written HCL text generation, never a parser --
//! see MIGRATION_PLAN.md #8 and #10 (no HCL-parsing crate; this module only
//! ever emits text). Output is a small file bundle, because a real
//! Terraform root always is one: `README.md`, `provider.tf`, `variables.tf`,
//! `main.tf`, `outputs.tf`.
//!
//! WHAT THIS IS HONEST ABOUT. A minimal Terraform mapping for "a database"
//! or "a queue" is a real, defensible starting point. A minimal mapping for
//! "a rate limiter" or "an autoscaler" is not, because those are policies
//! and controllers layered onto other infrastructure, not resources a cloud
//! provider provisions on their own -- see `mapped` below and the module's
//! own doc comment on which `NodeKind`s get a placeholder instead of a
//! guess. The same honesty governs every individual mapping: where a vendor
//! genuinely has no clean single-resource equivalent (GCP Cloud CDN is a
//! flag on a backend service, not a standalone resource type), this emits
//! the `MANUAL CONFIGURATION REQUIRED` placeholder rather than a
//! resource block that looks complete but silently does the wrong thing.

use std::collections::HashSet;

use crate::sim::types::NodeKind;
use crate::sysdesign::model::SystemDesignDoc;
use crate::vendors::types::VendorId;

/// Build the Terraform export bundle for `doc`. Always returns the same five
/// files in the same order; when no vendor is chosen (or "generic", which is
/// not a provisionable cloud) `provider.tf`, `main.tf` and `outputs.tf` are
/// comment-only rather than guessed.
pub fn export(doc: &SystemDesignDoc) -> Vec<(String, String)> {
    let vendor = doc.low_level.deployment.vendor;
    let (main_body, manual, emitted) = main_tf(doc, vendor);
    vec![
        ("README.md".to_string(), readme(doc, vendor, &manual)),
        ("provider.tf".to_string(), provider_tf(vendor)),
        ("variables.tf".to_string(), variables_tf(doc)),
        ("main.tf".to_string(), main_body),
        ("outputs.tf".to_string(), outputs_tf(vendor, &emitted)),
    ]
}

/// `None` (undecided) and `Some(VendorId::Generic)` (deliberately
/// vendor-neutral) are both "no cloud provider to generate HCL for" as far
/// as this module is concerned -- there is no Terraform provider for
/// "generic". `DeploymentConfig.vendor`'s doc comment explains why the two
/// stay distinct at the model layer even though they collapse here.
fn concrete_vendor(vendor: Option<VendorId>) -> Option<VendorId> {
    match vendor {
        Some(VendorId::Generic) | None => None,
        Some(v) => Some(v),
    }
}

fn vendor_label(vendor: VendorId) -> &'static str {
    match vendor {
        VendorId::Generic => "Generic",
        VendorId::Aws => "AWS",
        VendorId::Gcp => "GCP",
        VendorId::Azure => "Azure",
    }
}

/// Takes `kind` by reference deliberately: the caller in `main_tf` needs the
/// slug for both the mapping lookup and the comment header, and reading a
/// field twice out of a `&SimNode` should not have to depend on whether
/// `NodeKind` happens to derive `Copy`.
fn kind_slug(kind: &NodeKind) -> String {
    serde_json::to_value(kind)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_else(|| "unknown".to_string())
}

/// Escapes a value for placement inside a double-quoted HCL string literal.
fn hcl_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// A valid-enough HCL identifier derived from free text: lowercase ASCII
/// alphanumerics and underscores only, guaranteed to start with a letter.
fn sanitize_ident(s: &str) -> String {
    let mut out: String = s
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect();
    if out.is_empty() || !out.chars().next().unwrap().is_ascii_alphabetic() {
        out = format!("n_{out}");
    }
    out
}

/// GCP label values must be lowercase alphanumerics, `-` or `_`. Falls back
/// to a fixed placeholder rather than an empty label, which GCP rejects.
fn gcp_label_slug(s: &str) -> String {
    let slug: String = s
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let trimmed = slug.trim_matches('-');
    if trimmed.is_empty() {
        "node".to_string()
    } else {
        trimmed.to_string()
    }
}

/// Makes `base` unique against `used`, appending underscores as needed, and
/// records the result. Used for both resource local names and output names,
/// since two differently-labelled nodes can sanitize to the same identifier.
fn unique(base: String, used: &mut HashSet<String>) -> String {
    let mut candidate = base;
    while !used.insert(candidate.clone()) {
        candidate.push('_');
    }
    candidate
}

/* -------------------------------------------------------------------- *
 * The vendor + kind -> Terraform resource mapping table.
 *
 * Implemented for the eight kinds MIGRATION_PLAN.md #8 names explicitly:
 * Service, Db, Cache, ObjectStore, Queue, Lambda, Lb, Cdn -- across all
 * three vendors, with one deliberate exception (GCP Cdn, explained at its
 * match arm). Every other NodeKind falls through to `None` and gets the
 * MANUAL CONFIGURATION REQUIRED placeholder everywhere, including on every
 * vendor for kinds that are not independently provisionable resources at
 * all (Autoscaler, Breaker, RateLimiter, Bulkhead, LoadShedder, Sidecar are
 * policies/controllers layered onto other infrastructure, not something a
 * cloud provider stands up on its own; Client is the traffic source, not
 * infrastructure; Region is a multi-resource routing/failover concept, not
 * one resource). Extending this table to more kinds is straightforward but
 * was deliberately left undone here rather than guessed under time
 * pressure -- seven real mappings done honestly beats twenty done as a
 * guess dressed up as a fact.
 * -------------------------------------------------------------------- */

/// One resolved vendor+kind mapping.
struct Mapped {
    /// Terraform resource type, e.g. `"aws_instance"`.
    resource_type: &'static str,
    /// Attribute exposed as this resource's output value.
    output_attr: &'static str,
    /// HCL argument name for a size/SKU, when this resource type has one.
    /// `None` means `DeploymentResource.size_name` has nowhere to go and is
    /// noted with a comment instead of silently dropped.
    size_arg: Option<&'static str>,
    /// Extra required-argument lines this resource type cannot omit,
    /// already indented two spaces and newline-terminated, with `CHANGEME`
    /// placeholders for anything only the reader can fill in.
    extra_args: &'static str,
    /// Set when the mapping is real but does not by itself make a working
    /// resource (AWS's `aws_lb` still needs a listener and target group).
    /// Rendered as a `# NOTE:` comment inside the block.
    caveat: Option<&'static str>,
}

/// `kind` is the node's lowercase kind slug (`kind_slug`, e.g. `"service"`),
/// not a `NodeKind` value -- matching on the slug instead of the enum keeps
/// this table's lookup independent of whether `NodeKind` derives `Copy`,
/// since the caller already needs the slug for the comment header anyway.
fn mapped(vendor: VendorId, kind: &str) -> Option<Mapped> {
    use VendorId::*;
    match (vendor, kind) {
        // ---- AWS ----------------------------------------------------
        (Aws, "service") => Some(Mapped {
            resource_type: "aws_instance",
            output_attr: "id",
            size_arg: Some("instance_type"),
            extra_args: "  ami = \"CHANGEME-ami-id\" # TODO: pick an AMI for your OS and region\n",
            caveat: None,
        }),
        (Aws, "db") => Some(Mapped {
            resource_type: "aws_db_instance",
            output_attr: "endpoint",
            size_arg: Some("instance_class"),
            extra_args: "  engine              = \"postgres\" # TODO: match your actual engine\n  allocated_storage   = 20\n  username            = \"CHANGEME\"\n  password            = \"CHANGEME\" # TODO: use a secret manager, never a literal\n  skip_final_snapshot = true\n",
            caveat: None,
        }),
        (Aws, "cache") => Some(Mapped {
            resource_type: "aws_elasticache_cluster",
            output_attr: "cluster_address",
            size_arg: Some("node_type"),
            extra_args: "  engine          = \"redis\" # TODO: match your actual engine\n  num_cache_nodes = 1\n",
            caveat: None,
        }),
        (Aws, "objectstore") => Some(Mapped {
            resource_type: "aws_s3_bucket",
            output_attr: "bucket",
            size_arg: None,
            extra_args: "",
            caveat: None,
        }),
        (Aws, "queue") => Some(Mapped {
            resource_type: "aws_sqs_queue",
            output_attr: "url",
            size_arg: None,
            extra_args: "",
            caveat: None,
        }),
        (Aws, "lambda") => Some(Mapped {
            resource_type: "aws_lambda_function",
            output_attr: "arn",
            size_arg: None,
            extra_args: "  handler  = \"CHANGEME.handler\" # TODO: your function's entry point\n  runtime  = \"CHANGEME\" # TODO: e.g. \"nodejs20.x\"\n  filename = \"CHANGEME.zip\" # TODO: path to the deployment package\n  role     = \"CHANGEME-iam-role-arn\" # TODO: see README, no IAM was generated\n",
            caveat: None,
        }),
        (Aws, "lb") => Some(Mapped {
            resource_type: "aws_lb",
            output_attr: "dns_name",
            size_arg: None,
            extra_args: "  load_balancer_type = \"application\"\n  subnets            = [] # TODO: see README, no networking was generated\n",
            caveat: Some("needs at least one listener and target group before it routes any traffic"),
        }),
        (Aws, "cdn") => Some(Mapped {
            resource_type: "aws_cloudfront_distribution",
            output_attr: "domain_name",
            size_arg: None,
            extra_args: "  enabled = true\n",
            caveat: Some("needs an origin block and a default_cache_behavior block for your actual content"),
        }),

        // ---- GCP ------------------------------------------------------
        (Gcp, "service") => Some(Mapped {
            resource_type: "google_compute_instance",
            output_attr: "self_link",
            size_arg: Some("machine_type"),
            extra_args: "  zone = \"CHANGEME-zone\" # TODO: e.g. \"us-central1-a\"\n  boot_disk {\n    initialize_params {\n      image = \"CHANGEME-image\" # TODO: pick a boot image\n    }\n  }\n  network_interface {\n    network = \"default\" # TODO: see README, no networking was generated\n  }\n",
            caveat: None,
        }),
        (Gcp, "db") => Some(Mapped {
            resource_type: "google_sql_database_instance",
            output_attr: "connection_name",
            size_arg: Some("tier"),
            extra_args: "  database_version = \"POSTGRES_15\" # TODO: match your actual engine\n  region           = var.region\n",
            caveat: None,
        }),
        (Gcp, "cache") => Some(Mapped {
            resource_type: "google_redis_instance",
            output_attr: "host",
            size_arg: None,
            extra_args: "  tier           = \"BASIC\" # TODO: BASIC or STANDARD_HA\n  memory_size_gb = 1 # TODO: size for your workload\n  region         = var.region\n",
            caveat: None,
        }),
        (Gcp, "objectstore") => Some(Mapped {
            resource_type: "google_storage_bucket",
            output_attr: "url",
            size_arg: None,
            extra_args: "  location = var.region\n",
            caveat: None,
        }),
        (Gcp, "queue") => Some(Mapped {
            resource_type: "google_pubsub_topic",
            output_attr: "id",
            size_arg: None,
            extra_args: "",
            caveat: Some("Pub/Sub is a topic plus subscriptions; add a google_pubsub_subscription for each consumer"),
        }),
        (Gcp, "lambda") => Some(Mapped {
            resource_type: "google_cloudfunctions_function",
            output_attr: "https_trigger_url",
            size_arg: Some("available_memory_mb"),
            extra_args: "  runtime               = \"CHANGEME\" # TODO: e.g. \"nodejs20\"\n  entry_point           = \"CHANGEME\" # TODO: your function's entry point\n  source_archive_bucket = \"CHANGEME-bucket\" # TODO: see README, no bucket was generated\n  source_archive_object = \"CHANGEME.zip\"\n  trigger_http          = true\n",
            caveat: None,
        }),
        (Gcp, "lb") => Some(Mapped {
            resource_type: "google_compute_backend_service",
            output_attr: "self_link",
            size_arg: None,
            extra_args: "",
            caveat: Some("GCP load balancing is several resources; this backend service still needs a URL map, target proxy, forwarding rule and health check"),
        }),
        // GCP Cdn: deliberately unmapped. Cloud CDN is an `enable_cdn` flag
        // on a backend service, not a standalone resource type the way
        // aws_cloudfront_distribution or azurerm_cdn_endpoint are -- mapping
        // it to a single resource here would be the "plausible-looking
        // invented fact" AGENTS.md forbids, not a minimal starting point.

        // ---- Azure ------------------------------------------------------
        (Azure, "service") => Some(Mapped {
            resource_type: "azurerm_linux_virtual_machine",
            output_attr: "id",
            size_arg: Some("size"),
            extra_args: "  resource_group_name    = \"CHANGEME-resource-group\" # TODO: see README, none was generated\n  location               = var.region\n  admin_username         = \"CHANGEME\"\n  network_interface_ids  = [] # TODO: see README, no networking was generated\n  os_disk {\n    caching              = \"ReadWrite\"\n    storage_account_type = \"Standard_LRS\"\n  }\n  source_image_reference {\n    publisher = \"CHANGEME\"\n    offer     = \"CHANGEME\"\n    sku       = \"CHANGEME\"\n    version   = \"latest\"\n  }\n",
            caveat: None,
        }),
        (Azure, "db") => Some(Mapped {
            resource_type: "azurerm_mssql_database",
            output_attr: "id",
            size_arg: Some("sku_name"),
            extra_args: "  server_id = \"CHANGEME-mssql-server-id\" # TODO: see README, no server was generated\n",
            caveat: Some("azurerm_mssql_database assumes SQL Server; swap for azurerm_postgresql_flexible_server or similar if your engine differs"),
        }),
        (Azure, "cache") => Some(Mapped {
            resource_type: "azurerm_redis_cache",
            output_attr: "hostname",
            size_arg: None,
            extra_args: "  resource_group_name = \"CHANGEME-resource-group\"\n  location            = var.region\n  capacity            = 1 # TODO: size for your workload\n  family              = \"C\"\n  sku_name            = \"Basic\"\n",
            caveat: None,
        }),
        (Azure, "objectstore") => Some(Mapped {
            resource_type: "azurerm_storage_account",
            output_attr: "primary_blob_endpoint",
            size_arg: None,
            extra_args: "  resource_group_name      = \"CHANGEME-resource-group\"\n  location                 = var.region\n  account_tier             = \"Standard\"\n  account_replication_type = \"LRS\"\n",
            caveat: None,
        }),
        (Azure, "queue") => Some(Mapped {
            resource_type: "azurerm_servicebus_queue",
            output_attr: "id",
            size_arg: None,
            extra_args: "  namespace_id = \"CHANGEME-servicebus-namespace-id\" # TODO: see README, none was generated\n",
            caveat: None,
        }),
        (Azure, "lambda") => Some(Mapped {
            resource_type: "azurerm_function_app",
            output_attr: "default_hostname",
            size_arg: None,
            extra_args: "  resource_group_name         = \"CHANGEME-resource-group\"\n  location                    = var.region\n  app_service_plan_id         = \"CHANGEME-app-service-plan-id\" # TODO: see README, none was generated\n  storage_account_name        = \"CHANGEME-storage-account\"\n  storage_account_access_key  = \"CHANGEME\"\n",
            caveat: Some("azurerm_function_app is deprecated on newer azurerm provider versions in favor of azurerm_linux_function_app/azurerm_windows_function_app; swap if pinning a recent version"),
        }),
        (Azure, "lb") => Some(Mapped {
            resource_type: "azurerm_lb",
            output_attr: "id",
            size_arg: Some("sku"),
            extra_args: "  resource_group_name = \"CHANGEME-resource-group\"\n  location            = var.region\n",
            caveat: Some("needs a frontend IP configuration, backend pool and rules before it routes any traffic"),
        }),
        (Azure, "cdn") => Some(Mapped {
            resource_type: "azurerm_cdn_endpoint",
            output_attr: "fqdn",
            size_arg: None,
            extra_args: "  resource_group_name = \"CHANGEME-resource-group\"\n  location            = var.region\n  profile_name        = \"CHANGEME-cdn-profile-name\" # TODO: see README, no azurerm_cdn_profile was generated\n  origin {\n    name      = \"origin\"\n    host_name = \"CHANGEME-origin-hostname\"\n  }\n",
            caveat: None,
        }),

        _ => None,
    }
}

/// Render one resource's HCL block: a real minimal resource for a mapped
/// vendor+kind, or a `# MANUAL CONFIGURATION REQUIRED` comment block when
/// there is none. Returns the block text plus, only when a real resource
/// was emitted, its address (`"<type>.<local_name>"`) and output attribute.
fn resource_for(
    vendor: VendorId,
    kind_slug: &str,
    node_label: &str,
    local_name: &str,
    size_name: Option<&str>,
) -> (String, Option<(String, &'static str)>) {
    match mapped(vendor, kind_slug) {
        Some(m) => {
            let mut out = format!("resource \"{}\" \"{}\" {{\n", m.resource_type, local_name);
            if let Some(caveat) = m.caveat {
                out.push_str(&format!("  # NOTE: {caveat}\n"));
            }
            out.push_str(m.extra_args);
            match m.size_arg {
                Some(arg) => match size_name.filter(|s| !s.trim().is_empty()) {
                    Some(sz) => out.push_str(&format!("  {arg} = \"{}\"\n", hcl_escape(sz))),
                    None => out.push_str(&format!(
                        "  {arg} = \"CHANGEME\" # TODO: no size was set on this node in the Deployment section\n"
                    )),
                },
                None => {
                    if let Some(sz) = size_name.filter(|s| !s.trim().is_empty()) {
                        out.push_str(&format!(
                            "  # size \"{}\" was noted on this node but {} has no matching argument; set the right field by hand if one applies\n",
                            hcl_escape(sz),
                            m.resource_type
                        ));
                    }
                }
            }
            match vendor {
                VendorId::Gcp => out.push_str(&format!(
                    "  labels = {{\n    name = \"{}\"\n  }}\n",
                    gcp_label_slug(node_label)
                )),
                VendorId::Aws | VendorId::Azure => out.push_str(&format!(
                    "  tags = {{\n    Name = \"{}\"\n  }}\n",
                    hcl_escape(node_label)
                )),
                VendorId::Generic => {}
            }
            out.push_str("}\n");
            (
                out,
                Some((format!("{}.{}", m.resource_type, local_name), m.output_attr)),
            )
        }
        None => {
            let block = format!(
                "# MANUAL CONFIGURATION REQUIRED: no default Terraform mapping for \"{}\" on {}.\n# Configure this resource by hand; see the checklist in README.md for the node \"{}\".\n",
                kind_slug,
                vendor_label(vendor),
                node_label,
            );
            (block, None)
        }
    }
}

fn provider_tf(vendor: Option<VendorId>) -> String {
    match concrete_vendor(vendor) {
        None => "# Terraform export needs a vendor chosen in this design's Deployment section\n\
                 # before it can generate a provider block. Open the Deployment tab, pick AWS,\n\
                 # GCP or Azure, and export again. See README.md.\n"
            .to_string(),
        Some(v) => {
            let (source, provider_name) = match v {
                VendorId::Aws => ("hashicorp/aws", "aws"),
                VendorId::Gcp => ("hashicorp/google", "google"),
                VendorId::Azure => ("hashicorp/azurerm", "azurerm"),
                VendorId::Generic => unreachable!("concrete_vendor never returns Generic"),
            };
            let provider_block = match v {
                VendorId::Aws => "provider \"aws\" {\n  region = var.region\n}\n".to_string(),
                VendorId::Gcp => {
                    "provider \"google\" {\n  project = \"CHANGEME-project-id\" # TODO: set your GCP project id\n  region  = var.region\n}\n"
                        .to_string()
                }
                VendorId::Azure => "provider \"azurerm\" {\n  features {}\n}\n".to_string(),
                VendorId::Generic => unreachable!("concrete_vendor never returns Generic"),
            };
            format!(
                "terraform {{\n  required_version = \">= 1.5\"\n\n  required_providers {{\n    {provider_name} = {{\n      source  = \"{source}\"\n      version = \">= 1.0\" # TODO: pin to a tested version -- see README.md\n    }}\n  }}\n}}\n\n{provider_block}"
            )
        }
    }
}

fn variables_tf(doc: &SystemDesignDoc) -> String {
    let region = doc
        .low_level
        .deployment
        .region
        .as_deref()
        .filter(|r| !r.trim().is_empty())
        .unwrap_or("CHANGEME-region");
    let environment = {
        let e = doc.low_level.deployment.environment.trim();
        if e.is_empty() {
            "development"
        } else {
            e
        }
    };
    format!(
        "variable \"region\" {{\n  description = \"Deployment region.\"\n  type        = string\n  default     = \"{}\"\n}}\n\nvariable \"environment\" {{\n  description = \"Deployment environment name.\"\n  type        = string\n  default     = \"{}\"\n}}\n",
        hcl_escape(region),
        hcl_escape(environment),
    )
}

/// Builds `main.tf`. Returns the file text, the labels of nodes that got a
/// `MANUAL CONFIGURATION REQUIRED` placeholder, and the `(label, address,
/// output_attr)` of every resource that was actually emitted (for `outputs.tf`
/// and `README.md`).
fn main_tf(
    doc: &SystemDesignDoc,
    vendor: Option<VendorId>,
) -> (String, Vec<String>, Vec<(String, String, &'static str)>) {
    let Some(v) = concrete_vendor(vendor) else {
        return (
            "# Terraform export needs a vendor chosen in this design's Deployment section\n\
             # before resources can be generated. See provider.tf and README.md.\n"
                .to_string(),
            Vec::new(),
            Vec::new(),
        );
    };

    if doc.low_level.deployment.resources.is_empty() {
        return (
            "# This design has no deployment resources recorded yet. Add one per node\n\
             # that needs provisioned infrastructure in the Deployment section, then\n\
             # export again.\n"
                .to_string(),
            Vec::new(),
            Vec::new(),
        );
    }

    let mut body = String::new();
    let mut manual = Vec::new();
    let mut emitted = Vec::new();
    let mut used_names: HashSet<String> = HashSet::new();

    for resource in &doc.low_level.deployment.resources {
        let Some(node) = doc.topology.nodes.iter().find(|n| n.id == resource.node_id) else {
            // `validate::validate` already flags a resource pointing at a
            // node that no longer exists; main.tf just skips it rather than
            // emitting infrastructure for a node nobody can see any more.
            continue;
        };
        let local_name = unique(sanitize_ident(&node.id), &mut used_names);
        let slug = kind_slug(&node.kind);
        let (block, meta) = resource_for(
            v,
            &slug,
            &node.label,
            &local_name,
            resource.size_name.as_deref(),
        );
        body.push_str(&format!("# {} ({slug})\n", node.label));
        body.push_str(&block);
        body.push('\n');
        match meta {
            Some((address, output_attr)) => emitted.push((node.label.clone(), address, output_attr)),
            None => manual.push(node.label.clone()),
        }
    }

    (body, manual, emitted)
}

fn outputs_tf(vendor: Option<VendorId>, emitted: &[(String, String, &'static str)]) -> String {
    if concrete_vendor(vendor).is_none() {
        return "# No outputs: Terraform export needs a vendor chosen first. See provider.tf.\n"
            .to_string();
    }
    if emitted.is_empty() {
        return "# No outputs: none of this design's deployment resources had a default\n\
                 # Terraform mapping for the chosen vendor. See main.tf and README.md.\n"
            .to_string();
    }
    let mut out = String::new();
    let mut used = HashSet::new();
    for (label, address, attr) in emitted {
        let name = unique(sanitize_ident(label), &mut used);
        let description = hcl_escape(&format!("{attr} of the resource generated for \"{label}\"."));
        out.push_str(&format!(
            "output \"{name}\" {{\n  description = \"{description}\"\n  value       = {address}.{attr}\n}}\n\n"
        ));
    }
    out
}

fn readme(doc: &SystemDesignDoc, vendor: Option<VendorId>, manual: &[String]) -> String {
    let mut out = String::new();
    out.push_str(&format!("# Terraform export: {}\n\n", doc.name));
    out.push_str(&format!(
        "Generated {} from this system design. This is a STARTING POINT, not a\n\
         deployable stack. Read this whole file before you run `terraform plan`.\n\n",
        crate::util::iso8601::now()
    ));

    out.push_str("## What was not generated\n\n");
    out.push_str("- No VPC or networking of any kind (subnets, security groups, routing).\n");
    out.push_str("- No IAM roles or policies.\n");
    out.push_str("- No DNS.\n");
    out.push_str(
        "- No secrets management. Every password/key placeholder in `main.tf` is literal\n  text (`\"CHANGEME\"`) and must be replaced with a reference to a real secret\n  store before this is ever applied.\n",
    );
    out.push_str(
        "- No Terraform state backend. Add a `backend` block to `provider.tf` yourself\n  before running this anywhere but a local sandbox.\n\n",
    );

    match concrete_vendor(vendor) {
        None => {
            out.push_str("## No vendor chosen\n\n");
            out.push_str(
                "This design's Deployment section has no vendor set (or has \"Generic\", which\nis not a provisionable cloud), so `provider.tf`, `main.tf` and `outputs.tf` were\nleft as comment-only files. Pick AWS, GCP or Azure in the Deployment tab and\nexport again.\n\n",
            );
        }
        Some(v) => {
            out.push_str("## Provider version\n\n");
            out.push_str(&format!(
                "The `required_providers` version constraint in `provider.tf` is a placeholder\n(`>= 1.0`). Pin it to a version you have actually tested against this config\nbefore relying on it; {} provider behavior can change across major versions.\n\n",
                vendor_label(v)
            ));
        }
    }

    if manual.is_empty() {
        out.push_str("## Manual configuration\n\n");
        out.push_str(
            "Every deployment resource in this design had a default Terraform mapping for\nthe chosen vendor. That does not mean the generated blocks are complete --\nsee the `# NOTE:` comments inside `main.tf` for what each one still needs.\n\n",
        );
    } else {
        out.push_str("## Manual configuration required\n\n");
        out.push_str(
            "These nodes have a deployment resource but no default Terraform mapping for\nthe chosen vendor. `main.tf` contains a marked placeholder comment for each\none instead of a guessed resource; configure them by hand:\n\n",
        );
        for label in manual {
            out.push_str(&format!("- [ ] {label}\n"));
        }
        out.push('\n');
    }

    out.push_str("## Traceability\n\n");
    out.push_str(
        "Every generated resource carries the node's label from the design canvas, in\nits tags/labels and in the comment directly above it in `main.tf`, so you can\nmatch a resource back to the diagram.\n",
    );

    out
}
