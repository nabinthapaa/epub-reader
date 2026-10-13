# Folio — common tasks.

prefix := env_var_or_default("PREFIX", env_var("HOME") + "/.local")

# Build the reader in release mode.
build:
    cargo build --release

# Run it, optionally on a book:  just run ~/books/moby-dick.epub
run book="":
    cargo run --release -- {{book}}

test:
    cargo test --workspace

lint:
    cargo fmt --all --check
    cargo clippy --workspace --all-targets -- -D warnings

fmt:
    cargo fmt --all

# Install the binary, icon and desktop entry under PREFIX (default ~/.local).
install: build
    install -Dm755 target/release/folio {{prefix}}/bin/folio
    install -Dm644 packaging/folio.desktop {{prefix}}/share/applications/folio.desktop
    install -Dm644 packaging/folio.svg {{prefix}}/share/icons/hicolor/scalable/apps/folio.svg
    -update-desktop-database {{prefix}}/share/applications 2>/dev/null
    -xdg-mime default folio.desktop application/epub+zip
    @echo "installed to {{prefix}}/bin/folio — EPUB files now open in Folio"

uninstall:
    rm -f {{prefix}}/bin/folio
    rm -f {{prefix}}/share/applications/folio.desktop
    rm -f {{prefix}}/share/icons/hicolor/scalable/apps/folio.svg

# Render a page headlessly, for debugging layout:
#   just page ~/books/moby.epub "--chapter 3 --theme dark"
page book flags="":
    cargo run -p folio-core --example dump_page -- {{book}} {{flags}}

appimage: build
    packaging/appimage/build.sh

flatpak:
    flatpak-builder --force-clean --user --install build/flatpak packaging/flatpak/dev.folio.Folio.yml
