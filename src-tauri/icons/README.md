# Icons

The full icon set checked in here was generated once from
`static/favicon.svg`:

```bash
bun x @tauri-apps/cli icon static/favicon.svg
```

That covers everything `src-tauri/tauri.conf.json`'s `bundle.icon` array
references (`32x32.png`, `128x128.png`, `128x128@2x.png`, `icon.icns`,
`icon.ico`) plus the Windows Store, Android and iOS sets, so `tauri build`
and `tauri dev` can bundle without a preparatory step.

Re-run the command above after changing the source art. The favicon is
valid placeholder art, but a dedicated square app icon (1024x1024 PNG or
SVG, transparent background) usually looks better on a desktop launcher —
swap the source and regenerate rather than editing these binaries.
