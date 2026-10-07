/**
 * `.breakscale` design-file shapes.
 *
 * Mirrors `src/designFile.ts` from the web app. The actual build/parse/
 * validate logic lives in Rust now (`persistence::design_file`, invoked via
 * `api/designs.ts`'s `designFileBuild`/`designFileParse`/`designFileWrite`/
 * `designFileRead`) -- these are read-only mirrors for the frontend to
 * reference in UI copy, e.g. showing the file extension in a save dialog
 * filter or labelling a parse error. See MIGRATION_PLAN.md §6.
 */

import type { Topology } from './sim-types';

/**
 * Marker written into every exported file, and required on the way back in.
 * Not security, a courtesy: a student who opens the wrong JSON gets "this is
 * not a Breakscale design" instead of a wall of field complaints.
 */
export const DESIGN_FILE_APP = 'breakscale';

/**
 * Format version of the file body.
 *
 * Bumped only when the shape changes in a way an older reader could not
 * understand. A reader accepts its own version and anything below it; a file
 * from the future is refused with a message that says to update.
 */
export const DESIGN_FILE_VERSION = 1;

/**
 * The extension every exported design carries.
 *
 * A bare `.breakscale` rather than `.breakscale.json`: the contents are JSON
 * either way, but a custom extension is what an operating system can
 * associate with an application, and it reads as a document belonging to
 * this app rather than as a data file lying around.
 */
export const DESIGN_FILE_EXT = '.breakscale';

/** What one exported file holds. */
export interface DesignFile {
  app: typeof DESIGN_FILE_APP;
  version: number;
  /** When it was written, ISO 8601. Informational; nothing reads it back. */
  savedAt: string;
  /** The preset it started from, when it started from one. */
  name?: string;
  topology: Topology;
}

/**
 * A design that survived validation, or the reason it did not.
 *
 * A discriminated union rather than "null means no": every rejection carries
 * a cause a student can act on, matching the Rust `AppResult`-shaped JSON.
 */
export type DesignParseResult =
  | { ok: true; topology: Topology; name: string | null }
  | { ok: false; error: string };
