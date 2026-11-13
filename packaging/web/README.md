# Hosting LumenCraft for the Web

`lumencraft-web-<version>.zip` (built via `packaging/web/package.sh`) contains a self-contained
static site in `lumencraft-web-<version>/`:

| File | Description |
|---|---|
| `index.html` | Entry page using relative URLs (`public_url = "./"`), deployable at any path prefix. |
| `lumencraft-web-<hash>.js` | Generated `wasm-bindgen` ES module loader. |
| `lumencraft-web-<hash>_bg.wasm` | Compiled LumenCraft WebAssembly binary (`+simd128`). |
| `_headers`, `.htaccess` | Sample header configurations for Cloudflare Pages / Netlify and Apache. |

## Server Configuration

1. **MIME types:** Serve `.wasm` as `application/wasm` and `.js` as `text/javascript`.
2. **Compression:** Enable Brotli or gzip compression on `.wasm`, `.js`, `.html`, and `.ttf`.
3. **Caching:** Serve hashed `.wasm`, `.js`, and `fonts/*` with `Cache-Control: public, max-age=31536000, immutable`, and `index.html` with `Cache-Control: no-cache`.
4. **HTTPS:** WebGPU and clipboard access require a secure context (`https://` or `http://localhost`).

## Browser Floor

LumenCraft compiles WASM with `+simd128` (`.cargo/config.toml`) and renders via WebGPU with automatic fallback to WebGL2:
- Chrome / Edge 91+
- Firefox 89+
- Safari 16.4+
