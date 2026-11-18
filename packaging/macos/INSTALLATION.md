# Installing CurveBook on macOS

CurveBook ships a single universal disk image for Apple Silicon (`arm64`) and Intel (`x86_64`) Macs running macOS 11 or later:

- `curvebook-<version>-macos-universal.dmg`

There is no separate macOS CLI ZIP archive (`R0.28`): the headless CLI and MCP server binary (`curvebook-cli`) lives inside `CurveBook.app/Contents/MacOS/` alongside the desktop executable.

## Install the app

1. Verify the downloaded `.dmg` checksum and build attestation ([download verification guide](../../docs/release/verifying-downloads.md)):
   ```sh
   gh attestation verify SHA256SUMS.txt --repo HauserGroup/curvebook \
     --signer-workflow HauserGroup/curvebook/.github/workflows/release.yml
   shasum -a 256 --check --ignore-missing SHA256SUMS.txt
   ```
2. Open `curvebook-<version>-macos-universal.dmg` and drag `CurveBook.app` to `Applications`.
3. Eject the disk image and open `CurveBook` from `Applications` or Launchpad.

Beta and later releases are signed with an Apple Developer ID certificate and notarized (`R8.10`), with tickets stapled to both the disk image and `CurveBook.app`. Historical Alpha prereleases were ad-hoc signed and required **System Settings → Privacy & Security → Open Anyway** on first launch.

## Use `curvebook-cli` from Terminal (`PATH`)

`curvebook-cli` is bundled at `/Applications/CurveBook.app/Contents/MacOS/curvebook-cli`.

To run it by name from any shell session, add the bundle binary directory to your `PATH` in `~/.zshrc` (default macOS shell) or `~/.bash_profile`:

```sh
echo 'export PATH="/Applications/CurveBook.app/Contents/MacOS:$PATH"' >> ~/.zshrc
source ~/.zshrc
curvebook-cli --version
```

If you prefer not to extend `PATH` for the entire app directory, create a symlink in a directory that is already on your `PATH`:

```sh
mkdir -p ~/.local/bin
ln -sf /Applications/CurveBook.app/Contents/MacOS/curvebook-cli ~/.local/bin/curvebook-cli
```

Ensure `~/.local/bin` is on your `PATH` (for example `export PATH="$HOME/.local/bin:$PATH"` in `~/.zshrc`). MCP clients such as Claude Desktop or IDE integrations can also invoke `/Applications/CurveBook.app/Contents/MacOS/curvebook-cli` directly by absolute path with arguments `["mcp"]`.
