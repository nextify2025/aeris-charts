# Aeris Charts for Rust

Aeris Charts is a high-performance financial chart engine with deterministic state, professional
interactions, drawings, indicators, backend-neutral frame construction, and native, WebGPU, and
WebAssembly rendering paths.

## Getting started

Use the headless engine directly:

```sh
cargo add aeris_charts_engine
```

Add the renderer required by the host:

```sh
cargo add aeris_charts_render_wgpu
# or
cargo add aeris_charts_native
```

The repository is preparing coordinated release `0.3.0`. All published `aeris_charts_*` crates in
a coordinated release use the same version. Previously published artifacts retain the license
bundled with their release.

## Crates

| Crate | Purpose |
| --- | --- |
| [`aeris_charts_engine`](https://crates.io/crates/aeris_charts_engine) | Chart state, interactions, drawings, indicators, and frame construction |
| [`aeris_charts_core`](https://crates.io/crates/aeris_charts_core) | Platform-free data, scales, options, validation, and formatting |
| [`aeris_charts_indicators`](https://crates.io/crates/aeris_charts_indicators) | Pure Rust technical-indicator calculations |
| [`aeris_charts_render`](https://crates.io/crates/aeris_charts_render) | Backend-neutral draw-list contract and rendering math |
| [`aeris_charts_render_wgpu`](https://crates.io/crates/aeris_charts_render_wgpu) | WebGPU executor |
| [`aeris_charts_native`](https://crates.io/crates/aeris_charts_native) | Native tiny-skia rasterizer and server-side PNG rendering |
| [`aeris_charts_wasm`](https://crates.io/crates/aeris_charts_wasm) | WebAssembly browser host |

The optional GPUI executor is available from the repository but is not published to crates.io. It
builds on `gpui-pre` 0.3.6, the GPUI snapshot that gpui-kit 0.6.6 pins, rather than the registry
`gpui` release.

## Publish order and deprecation

The prepared `0.3.0` crates are published in dependency order: `aeris_charts_indicators`,
`aeris_charts_core`, `aeris_charts_render`, `aeris_charts_engine`, `aeris_charts_render_wgpu`,
`aeris_charts_native`, then `aeris_charts_wasm`. `aeris_charts_render_gpui` remains
`publish = false`. This rename does not publish from this repository. Existing crates in the
legacy family should receive a final deprecation release that points to the matching
`aeris_charts_*` crate, then remain available for one coordinated migration window before being
marked deprecated. The former scoped npm package should receive the same notice and
redirect consumers to `@aeristerminal/aeris-charts`; keep its last release available for the migration window.

## Aeris Terminal platform follow-ups

The separate Aeris Terminal repository must update pinned Git revisions and Cargo package names
to the `aeris_charts_*` crates, replace workspace paths and Rust `use` imports, and update its npm
dependency and `/react` import from the legacy scoped npm package to `@aeristerminal/aeris-charts`. It must regenerate
lockfiles, update any WASM asset names and release automation, and replace repository URLs with
the `aeristerminal/aeris-charts` repository. No platform repository files are changed here.

## License

Aeris Charts is open-source software under the
[GNU Affero General Public License v3.0](https://github.com/aeristerminal/aeris-charts/blob/main/LICENSE),
identified by `AGPL-3.0-only`. A separate commercial license is available for proprietary,
OEM/embedded, and white-label use; see the
[commercial licensing notice](https://github.com/aeristerminal/aeris-charts/blob/main/COMMERCIAL_LICENSE.md).
