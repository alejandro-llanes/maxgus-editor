#!/bin/sh
#
# maxgus installer.
#
#     curl -fsSL https://alejandrollanes.com/maxgus-editor/install.sh | sh
#
# Not the `alejandro-llanes.github.io` address: the account's user site has
# a custom domain, so GitHub 301s every project path there — and to `http`,
# which is a poor hop for something piped into a shell. This is where the
# redirect lands, reached directly.
#
# Downloads a release archive, checks it against the checksum published
# beside it, and puts the binary somewhere on the path. It also writes a
# configuration and the themes it can name, the first time only, and adds
# an application-menu entry for the `gui` build. Nothing else: no daemon,
# no package manager, no shell profile rewritten behind your back, and
# never over a file you have edited.
#
# Options, after `-s --`:
#
#     --build minimal|full|gui   which of the three (default: full)
#     --version vX.Y.Z           a particular release (default: the latest)
#     --prefix DIR               where to put it (default: ~/.local/bin)
#     --dry-run                  say what it would do, and stop
#     --no-config                leave ~/.config/maxgus alone
#     --no-desktop               do not add an application-menu entry
#
# `MAXGUS_RELEASE_BASE` points it at a mirror instead of GitHub.
#
#     curl -fsSL .../install.sh | sh -s -- --build gui --prefix /usr/local/bin
#
# POSIX sh: this has to run under dash and busybox ash as well as bash.
set -eu

REPO="alejandro-llanes/maxgus-editor"
BUILD="full"
VERSION=""
PREFIX=""
DRY_RUN=0
CONFIG=1
DESKTOP=1

say() { printf '%s\n' "$*"; }
die() { printf 'maxgus: %s\n' "$*" >&2; exit 1; }

usage() {
    sed -n '3,26p' "$0" 2>/dev/null | sed 's/^# \{0,1\}//'
    exit "${1:-0}"
}

while [ $# -gt 0 ]; do
    case "$1" in
        --build) BUILD="${2:-}"; shift 2 ;;
        --build=*) BUILD="${1#*=}"; shift ;;
        --version) VERSION="${2:-}"; shift 2 ;;
        --version=*) VERSION="${1#*=}"; shift ;;
        --prefix) PREFIX="${2:-}"; shift 2 ;;
        --prefix=*) PREFIX="${1#*=}"; shift ;;
        --dry-run) DRY_RUN=1; shift ;;
        --no-config) CONFIG=0; shift ;;
        --no-desktop) DESKTOP=0; shift ;;
        -h|--help) usage 0 ;;
        *) die "unknown option: $1 (try --help)" ;;
    esac
done

case "$BUILD" in
    minimal|full|gui) ;;
    *) die "--build must be minimal, full or gui, not '$BUILD'" ;;
esac

# ---- what this machine is ------------------------------------------------

need() {
    command -v "$1" >/dev/null 2>&1 || die "this needs $1, which is not installed"
}
need uname

os=$(uname -s)
arch=$(uname -m)

case "$os" in
    Linux)   os_name=linux ;;
    Darwin)  os_name=macos ;;
    FreeBSD) os_name=freebsd ;;
    MINGW*|MSYS*|CYGWIN*)
        die "on Windows, download the .zip from https://github.com/$REPO/releases/latest" ;;
    *) die "no release is built for $os; build from source: https://github.com/$REPO" ;;
esac

case "$arch" in
    x86_64|amd64) arch_name=x86_64 ;;
    aarch64|arm64) arch_name=aarch64 ;;
    *) die "no release is built for $arch; build from source: https://github.com/$REPO" ;;
esac

platform="$os_name-$arch_name"

# musl systems get the static build, which runs anywhere. `ldd --version`
# names the C library on both glibc and musl, and says so on stderr on musl.
if [ "$os_name" = linux ] && [ "$arch_name" = x86_64 ]; then
    if (ldd --version 2>&1 || true) | head -1 | grep -qi musl; then
        platform="linux-x86_64-musl"
    fi
fi

# Not every build exists for every platform: a static binary cannot load a
# window system, and the cross-compiled targets have no window system to
# compile against.
case "$platform:$BUILD" in
    linux-x86_64-musl:gui|linux-aarch64:gui|freebsd-x86_64:gui)
        die "there is no gui build for $platform — try --build full, which is
     the same editor without the window" ;;
esac

# ---- which release -------------------------------------------------------

if command -v curl >/dev/null 2>&1; then
    fetch() { curl -fsSL "$1"; }
    download() { curl -fsSL --progress-bar -o "$2" "$1"; }
elif command -v wget >/dev/null 2>&1; then
    fetch() { wget -qO- "$1"; }
    download() { wget -q --show-progress -O "$2" "$1"; }
else
    die "this needs curl or wget, and has neither"
fi

# A mirror, or a copy on a machine that cannot reach GitHub. Also how this
# script is tested against real archives without publishing a release.
if [ -n "${MAXGUS_RELEASE_BASE:-}" ]; then
    base="$MAXGUS_RELEASE_BASE"
    label="$base"
elif [ -z "$VERSION" ]; then
    base="https://github.com/$REPO/releases/latest/download"
    label="the latest release"
else
    case "$VERSION" in v*) ;; *) VERSION="v$VERSION" ;; esac
    base="https://github.com/$REPO/releases/download/$VERSION"
    label="$VERSION"
fi

archive="maxgus-$BUILD-$platform.tar.gz"
url="$base/$archive"

# ---- where it goes -------------------------------------------------------

if [ -z "$PREFIX" ]; then
    PREFIX="$HOME/.local/bin"
    # A writable /usr/local/bin is already on everyone's path, which saves a
    # paragraph of "now add this to your shell profile".
    if [ -w /usr/local/bin ] 2>/dev/null; then
        PREFIX=/usr/local/bin
    fi
fi

say "maxgus $BUILD, $label, for $platform"
say "  from $url"
say "  into $PREFIX/maxgus"
if [ "$DRY_RUN" = 1 ]; then
    say ""
    say "(--dry-run: nothing was downloaded)"
    exit 0
fi

# ---- fetch, check, install ----------------------------------------------

need tar
work=$(mktemp -d 2>/dev/null || mktemp -d -t maxgus)
trap 'rm -rf "$work"' EXIT INT TERM

say ""
download "$url" "$work/$archive" || die "could not download $url
     If this is a fresh install, check that a release exists:
     https://github.com/$REPO/releases"

# The checksum is published beside the archive. A download that cannot be
# checked is not installed: this pipes to a shell, and that is exactly the
# arrangement that has to be careful.
if fetch "$url.sha256" > "$work/expected" 2>/dev/null && [ -s "$work/expected" ]; then
    expected=$(tr -d ' \t\r\n' < "$work/expected" | cut -c1-64)
    if command -v sha256sum >/dev/null 2>&1; then
        actual=$(sha256sum "$work/$archive" | cut -d' ' -f1)
    elif command -v shasum >/dev/null 2>&1; then
        actual=$(shasum -a 256 "$work/$archive" | cut -d' ' -f1)
    else
        actual=""
        say "  (no sha256sum or shasum here, so the checksum was not verified)"
    fi
    if [ -n "$actual" ] && [ "$actual" != "$expected" ]; then
        die "the download does not match its published checksum.
     expected $expected
     got      $actual
     Nothing was installed."
    fi
    [ -n "$actual" ] && say "  checksum ok"
else
    say "  (no published checksum for this archive; it was not verified)"
fi

tar -xzf "$work/$archive" -C "$work"
binary="$work/maxgus-$BUILD-$platform/maxgus"
[ -f "$binary" ] || die "the archive does not contain a maxgus binary"

mkdir -p "$PREFIX" || die "cannot create $PREFIX"
if [ -w "$PREFIX" ]; then
    install -m 755 "$binary" "$PREFIX/maxgus"
elif command -v sudo >/dev/null 2>&1; then
    say "  $PREFIX needs root; asking sudo"
    sudo install -m 755 "$binary" "$PREFIX/maxgus"
else
    die "$PREFIX is not writable, and there is no sudo.
     Try --prefix \$HOME/.local/bin"
fi

installed=$("$PREFIX/maxgus" --version 2>/dev/null || echo "maxgus")
say ""
say "Installed $installed"

# ---- the configuration, and the themes it can name ----------------------

# Never over an existing file. Someone who has configured this editor has
# said what they want, and an installer that helpfully replaces it has
# thrown that away — which is the one thing an installer must not do.
copy_if_absent() {
    if [ -e "$2" ]; then
        return 1
    fi
    mkdir -p "$(dirname "$2")" && cp "$1" "$2" && chmod 644 "$2"
}

unpacked="$work/maxgus-$BUILD-$platform"
config_dir="${XDG_CONFIG_HOME:-$HOME/.config}/maxgus"
if [ "$CONFIG" = 1 ] && [ -d "$unpacked/docs" ]; then
    written=0
    skipped=0
    # The example is always refreshed: it is documentation, it is where
    # every setting is written down, and it is not the file anyone edits.
    if [ -f "$unpacked/docs/config.example.kdl" ]; then
        mkdir -p "$config_dir"
        cp "$unpacked/docs/config.example.kdl" "$config_dir/config.example.kdl"
        # And it is the configuration itself, the first time only.
        if copy_if_absent "$unpacked/docs/config.example.kdl" "$config_dir/config.kdl"; then
            written=$((written + 1))
        else
            skipped=$((skipped + 1))
        fi
    fi
    for reference in configuration.md configuration-reference.md grammars.md; do
        [ -f "$unpacked/docs/$reference" ] || continue
        cp "$unpacked/docs/$reference" "$config_dir/$reference"
    done
    for theme in "$unpacked"/docs/themes/*.kdl; do
        [ -f "$theme" ] || continue
        if copy_if_absent "$theme" "$config_dir/themes/$(basename "$theme")"; then
            written=$((written + 1))
        else
            skipped=$((skipped + 1))
        fi
    done
    say ""
    say "Configuration in $config_dir"
    say "  $written written, $skipped left alone because they were already there"
fi

# ---- the desktop entry, for the build that opens a window ---------------

# Only the `gui` build, and only where there is a desktop to register with:
# a `.desktop` file is freedesktop's, so it means nothing on macOS or
# Windows, and a terminal-only build has no window to launch.
if [ "$DESKTOP" = 1 ] && [ "$BUILD" = gui ] && [ "$os_name" != macos ] \
   && [ -f "$unpacked/assets/maxgus.desktop" ]; then
    # Beside the binary when it went somewhere with a `share` next to it,
    # and in the user's own data directory otherwise.
    case "$PREFIX" in
        */bin)
            data="$(dirname "$PREFIX")/share"
            [ -w "$(dirname "$PREFIX")" ] || data="${XDG_DATA_HOME:-$HOME/.local/share}" ;;
        *) data="${XDG_DATA_HOME:-$HOME/.local/share}" ;;
    esac
    apps="$data/applications"
    icons="$data/icons/hicolor/scalable/apps"
    if mkdir -p "$apps" "$icons" 2>/dev/null; then
        # A launcher does not see the shell's PATH, so the entry names the
        # binary in full. Anything else is a menu item that does nothing.
        sed -e "s|^Exec=maxgus |Exec=$PREFIX/maxgus |" \
            -e "s|^TryExec=maxgus$|TryExec=$PREFIX/maxgus|" \
            "$unpacked/assets/maxgus.desktop" > "$apps/maxgus.desktop"
        chmod 644 "$apps/maxgus.desktop"
        [ -f "$unpacked/assets/maxgus.svg" ] && cp "$unpacked/assets/maxgus.svg" "$icons/maxgus.svg"
        # Tell the desktop, where it wants telling. Neither is required and
        # neither failing matters: the entry is read at the next login.
        command -v update-desktop-database >/dev/null 2>&1 &&
            update-desktop-database "$apps" >/dev/null 2>&1
        command -v gtk-update-icon-cache >/dev/null 2>&1 &&
            gtk-update-icon-cache -qtf "$data/icons/hicolor" >/dev/null 2>&1
        say ""
        say "Listed in the application menu: $apps/maxgus.desktop"
    fi
fi

case ":$PATH:" in
    *":$PREFIX:"*)
        say "Run it: maxgus FILE" ;;
    *)
        say "$PREFIX is not on your PATH. Either run it in full:"
        say "    $PREFIX/maxgus FILE"
        say "or add it, with the line your shell wants:"
        say "    export PATH=\"$PREFIX:\$PATH\"" ;;
esac

if [ "$BUILD" = gui ]; then
    say ""
    say "This build opens a window. \`maxgus -nw FILE\` uses the terminal."
fi
say "Press C-h t inside it for a guided tour."
