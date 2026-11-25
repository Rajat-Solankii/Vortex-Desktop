# Vortex Desktop

**Vortex Desktop is licensed under the GNU General Public License, version 3 only (`GPL-3.0-only`).** The full license is in [LICENSE](LICENSE). Third-party assets and dependencies retain their own licenses.

A native Rust desktop app based on the streaming catalog of the Vortex website. It uses egui/eframe for Windows, Linux (X11 and Wayland), and macOS (Apple silicon and Intel).

The website-inspired layout uses Geist, a black top navigation bar, full-width featured artwork, and cyan-accented poster rows. Use the top bar to browse Anime, Movies, TV Shows, and Genres. Open the Search panel with Ctrl+K on Windows/Linux or ⌘K on macOS. Submitted searches and searches used to open a result appear in local Recent Searches.

Home, category browsing, search, title details, and episode lists use the Vortex catalog API. Library saves to the current desktop installation. Settings controls carousel autoplay and poster size. Watch plays inside the title page, with a server selector above the player. TV pages have season controls and a separately scrolling episode grid. Title pages show cast, trailers, and recommendations when available. Opening an upcoming title shows a Coming Soon dialog with its release date when known. The Linux player uses X11 or XWayland. Continue Watching is local and resumes the last episode started for a title. Vortex Shield blocks known ad and tracking requests in the native stream webview and prevents pop-ups and off-list navigation. Provider catalogs show US availability.

Use **Fullscreen** by the player to expand it, then **Exit fullscreen** to return to the title page. **Cast screen** opens the Windows wireless display picker, macOS Displays settings for AirPlay, or GNOME Network Displays on Linux. Screen mirroring requires a compatible receiver and, on Linux, `gnome-network-displays`.

## Build and run

Install a current stable Rust toolchain.

- **Windows** requires the MSVC C++ build tools and Windows SDK. Playback requires the Microsoft Edge WebView2 Runtime.
- **macOS** requires the Xcode command-line tools (`xcode-select --install`).
- **Debian/Ubuntu** requires a working OpenGL driver and desktop libraries. Install the build prerequisites with:

  ```sh
  sudo apt-get install build-essential pkg-config libgl1-mesa-dev libgtk-3-dev libwebkit2gtk-4.1-dev libwayland-dev libxkbcommon-dev libxkbcommon-x11-0 libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev
  ```

On Debian/Ubuntu, the release executable needs the GTK 3, WebKitGTK 4.1, and XKB X11 runtime libraries (`libgtk-3-0`, `libwebkit2gtk-4.1-0`, and `libxkbcommon-x11-0`, or their distribution equivalents). The player uses GStreamer. Install the GStreamer plugins for your media codecs if playback reports a missing decoder.

Set `VORTEX_CATALOG_API_URL` in your shell to the full HTTPS catalog endpoint, then run from the desktop project directory. Cargo embeds the URL at compile time. The app does not require the environment variable at runtime.

```sh
cargo run --locked
```

## Build release packages

Run the build script from the project directory. On Windows, use Git Bash with the MSVC build tools installed.

```sh
bash scripts/build-desktop.sh
```

The script runs the Rust tests with the locked dependencies, builds the release, and saves a `.tar.gz` archive under `target/packages`. The archive includes the executable or macOS `Vortex.app`, the GPL license, and the Geist license. macOS app bundles are ad hoc signed.

The [Desktop builds workflow](.github/workflows/desktop.yml) uses the same script on pushes to `main`. You can also start it manually on `main` from **Actions → Desktop builds → Run workflow**. The workflow builds Windows x64, Linux x64, macOS Apple silicon, and macOS Intel. Download the packages from the run’s **Artifacts** section. Builds from a public repository use a placeholder endpoint, so those packages cannot load the catalog. For a package with catalog access, build locally with `VORTEX_CATALOG_API_URL` set or configure that Actions secret in a private repository.

The build setting keeps the URL out of the source files. The installed app’s network requests can reveal the destination, so API access control must be enforced by the server.

Set `VORTEX_ACCOUNT_API_URL` to the HTTPS account service URL ending in `/v1/account` to enable the account panel in Settings. This value can be set at build time or in the launch environment. The panel supports signing in, password reset, and manually saving or restoring user data.

## Third-party attribution

Vortex Desktop incorporates its app icon from Vortex Mobile, which is built on Nuvio Mobile. The bundled Geist font is covered by the [SIL Open Font License](assets/geist-OFL.txt).
