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
# beside it, makes sure the binary in it runs here, and puts it somewhere on
# the path. The first time, it also writes a short configuration and the
# themes it can name, and adds an application-menu entry for the `gui`
# build. Nothing else: no daemon, no package manager, no shell profile
# rewritten behind your back, and never over your configuration or a theme
# you have edited.
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
CHECKSUM=1

say() { printf '%s\n' "$*"; }
die() { printf 'maxgus: %s\n' "$*" >&2; exit 1; }

# The usage is written out here rather than read back from this file's
# comments: piped into a shell, this script has no file to read them from,
# and `--help` printed nothing — or the shell's own binary.
usage() {
    cat <<'EOF'
maxgus installer

    curl -fsSL https://alejandrollanes.com/maxgus-editor/install.sh | sh -s -- [options]

Options:

    --build minimal|full|gui   which of the three (default: full)
    --version vX.Y.Z           a particular release (default: the latest)
    --prefix DIR               where to put it (default: /usr/local/bin when
                               that is writable, ~/.local/bin otherwise)
    --dry-run                  say what it would do, and stop
    --no-config                leave the configuration directory alone
    --no-desktop               do not add an application-menu entry
    --insecure-skip-checksum   install even when the download cannot be
                               checked against its published checksum

MAXGUS_RELEASE_BASE points it at a mirror instead of GitHub.
EOF
    exit "${1:-0}"
}

# An option that takes a value, and was not given one.
value() {
    [ "$2" -ge 2 ] || die "$1 needs a value (try --help)"
}

while [ $# -gt 0 ]; do
    case "$1" in
        --build) value "$1" $#; BUILD="$2"; shift 2 ;;
        --build=*) BUILD="${1#*=}"; shift ;;
        --version) value "$1" $#; VERSION="$2"; shift 2 ;;
        --version=*) VERSION="${1#*=}"; shift ;;
        --prefix) value "$1" $#; PREFIX="$2"; shift 2 ;;
        --prefix=*) PREFIX="${1#*=}"; shift ;;
        --dry-run) DRY_RUN=1; shift ;;
        --no-config) CONFIG=0; shift ;;
        --no-desktop) DESKTOP=0; shift ;;
        --insecure-skip-checksum) CHECKSUM=0; shift ;;
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
musl=0
if [ "$os_name" = linux ] && (ldd --version 2>&1 || true) | head -1 | grep -qi musl; then
    musl=1
fi
if [ "$musl" = 1 ] && [ "$arch_name" = x86_64 ]; then
    platform="linux-x86_64-musl"
fi

# What is built, said before anything is downloaded: asking for one that
# does not exist was only found out by a download failing, and blamed on a
# release that might not exist.
case "$platform" in
    linux-x86_64|linux-x86_64-musl|linux-aarch64|macos-x86_64|macos-aarch64|freebsd-x86_64) ;;
    *) die "no release is built for $platform; build from source: https://github.com/$REPO" ;;
esac
if [ "$musl" = 1 ] && [ "$arch_name" = aarch64 ]; then
    die "the aarch64 builds need glibc, and this system uses musl; build from source:
     https://github.com/$REPO"
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
    download() { curl -fSL --progress-bar -o "$2" "$1"; }
elif command -v wget >/dev/null 2>&1; then
    fetch() { wget -qO- "$1"; }
    # busybox's wget has no `--show-progress`, and refused to download
    # anything when it was given one.
    if wget --help 2>&1 | grep -q -- --show-progress; then
        download() { wget -q --show-progress -O "$2" "$1"; }
    else
        download() { wget -q -O "$2" "$1"; }
    fi
elif command -v fetch >/dev/null 2>&1; then
    # FreeBSD's own, which is there when neither of the others is.
    fetch() { command fetch -qo - "$1"; }
    download() { command fetch -o "$2" "$1"; }
else
    die "this needs curl, wget or fetch, and has none of them"
fi

case "$VERSION" in
    ""|v*) ;;
    *) VERSION="v$VERSION" ;;
esac
# Before 0.2.0 the archives were named differently, and no name this can
# build would be found.
case "$VERSION" in
    v0.0.*|v0.1.*)
        die "releases before v0.2.0 are not packaged the way this installs; download
     one by hand from https://github.com/$REPO/releases" ;;
esac

# A mirror, or a copy on a machine that cannot reach GitHub. Also how this
# script is tested against real archives without publishing a release.
if [ -n "${MAXGUS_RELEASE_BASE:-}" ]; then
    base="$MAXGUS_RELEASE_BASE"
    label="$base"
    [ -z "$VERSION" ] || say "(MAXGUS_RELEASE_BASE is set, so --version $VERSION is not used)"
elif [ -z "$VERSION" ]; then
    base="https://github.com/$REPO/releases/latest/download"
    label="the latest release"
else
    base="https://github.com/$REPO/releases/download/$VERSION"
    label="$VERSION"
fi

# ---- where it goes -------------------------------------------------------

if [ -z "$PREFIX" ]; then
    PREFIX="$HOME/.local/bin"
    # A writable /usr/local/bin is already on everyone's path, which saves a
    # paragraph of "now add this to your shell profile".
    if [ -w /usr/local/bin ] 2>/dev/null; then
        PREFIX=/usr/local/bin
    fi
fi
# In full: the desktop entry names the binary by this, and a launcher does
# not start in the directory the installer was run from.
case "$PREFIX" in
    /*) ;;
    *) PREFIX="$(pwd)/$PREFIX" ;;
esac

# Where the editor reads its configuration. On macOS that is not
# `~/.config`, whatever `XDG_CONFIG_HOME` says, and a configuration put there
# was never read.
case "$os_name" in
    macos) config_dir="$HOME/Library/Application Support/maxgus" ;;
    *) config_dir="${XDG_CONFIG_HOME:-$HOME/.config}/maxgus" ;;
esac

archive_for() { printf 'maxgus-%s-%s.tar.gz' "$BUILD" "$1"; }
archive=$(archive_for "$platform")
url="$base/$archive"

say "maxgus $BUILD, $label, for $platform"
say "  from $url"
say "  into $PREFIX/maxgus"
if [ "$CONFIG" = 1 ]; then
    say "  a configuration in $config_dir, if there is none"
fi
if [ "$DESKTOP" = 1 ] && [ "$BUILD" = gui ] && [ "$os_name" != macos ]; then
    say "  an application-menu entry"
fi
if [ "$DRY_RUN" = 1 ]; then
    say ""
    say "(--dry-run: nothing was downloaded)"
    exit 0
fi

# ---- fetch, check, install ----------------------------------------------

need tar
work=$(mktemp -d 2>/dev/null || mktemp -d -t maxgus)
trap 'rm -rf "$work"' EXIT
# Interrupted, it stops: the trap used to clean up and carry straight on.
trap 'exit 130' INT TERM

# The SHA-256 of a file, by whichever tool this system has.
sha256_of() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | cut -d' ' -f1
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$1" | cut -d' ' -f1
    elif command -v sha256 >/dev/null 2>&1; then
        sha256 -q "$1"
    elif command -v openssl >/dev/null 2>&1; then
        openssl dgst -sha256 "$1" | sed 's/.*= *//'
    fi
}

# Downloads `$1` into the work directory, checks it against its published
# checksum and unpacks it.
#
# A download that cannot be checked is not installed: this is piped into a
# shell, and that is exactly the arrangement that has to be careful. It was
# installed anyway, with a note, when there was no checksum or nothing to
# compute one with.
fetch_archive() {
    download "$base/$1" "$work/$1" || die "could not download $base/$1
     If this is a fresh install, check that a release exists:
     https://github.com/$REPO/releases"
    if [ "$CHECKSUM" = 1 ]; then
        fetch "$base/$1.sha256" > "$work/$1.sha256" 2>/dev/null && [ -s "$work/$1.sha256" ] ||
            die "there is no published checksum for $1, so it cannot be checked.
     Nothing was installed. --insecure-skip-checksum installs it regardless."
        expected=$(cut -c1-64 < "$work/$1.sha256")
        actual=$(sha256_of "$work/$1")
        [ -n "$actual" ] ||
            die "there is no sha256sum, shasum, sha256 or openssl here to check the download with.
     Nothing was installed. --insecure-skip-checksum installs it regardless."
        [ "$actual" = "$expected" ] || die "the download does not match its published checksum.
     expected $expected
     got      $actual
     Nothing was installed."
        say "  checksum ok"
    else
        say "  (--insecure-skip-checksum: the download was not checked)"
    fi
    tar -xzf "$work/$1" -C "$work"
}

say ""
fetch_archive "$archive"
unpacked="$work/maxgus-$BUILD-$platform"
binary="$unpacked/maxgus"
[ -f "$binary" ] || die "the archive does not contain a maxgus binary"

# A binary that will not start here is not installed. The Linux builds need
# the C library they were built against or a newer one, and one that could
# not load was installed and reported as "Installed maxgus".
if ! installed=$("$binary" --version 2>"$work/refusal"); then
    if [ "$platform" = linux-x86_64 ] && [ "$BUILD" != gui ]; then
        say "  that build does not run here ($(head -1 "$work/refusal")); the static one will"
        platform="linux-x86_64-musl"
        archive=$(archive_for "$platform")
        fetch_archive "$archive"
        unpacked="$work/maxgus-$BUILD-$platform"
        binary="$unpacked/maxgus"
        [ -f "$binary" ] || die "the archive does not contain a maxgus binary"
        installed=$("$binary" --version 2>"$work/refusal") ||
            die "the binary does not run here: $(head -1 "$work/refusal")"
    else
        die "the binary does not run here: $(head -1 "$work/refusal")
     Nothing was installed.$( [ "$os_name" = linux ] && printf '%s' "
     The Linux builds need glibc 2.35 or newer; --build full uses a static
     build that does not." )"
    fi
fi

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

if [ "$CONFIG" = 1 ] && [ -d "$unpacked/docs" ]; then
    mkdir -p "$config_dir"
    # A short configuration, not the example. The example sets most of what
    # there is to set — line numbers, trailing whitespace stripped on every
    # save, a shell, grammars loaded from the system — and as a first
    # configuration it made an editor that behaved like nothing the
    # documentation describes.
    if [ -e "$config_dir/config.kdl" ]; then
        config_said="config.kdl left alone"
    else
        cat > "$config_dir/config.kdl" <<'EOF'
// maxgus configuration, in KDL. Nothing has to be here: the defaults are
// what the documentation describes. C-c f p opens this file from inside the
// editor.
//
// Every setting, with its default, is in config.example.kdl beside this
// file, and explained in configuration-reference.md.
//
// set theme="maxgus-dark"
// set line-numbers=#true
EOF
        chmod 644 "$config_dir/config.kdl"
        config_said="config.kdl written"
    fi
    # The example and the references are refreshed every time: they are
    # documentation of the version just installed, not files anyone edits.
    refreshed=0
    for reference in config.example.kdl configuration.md configuration-reference.md grammars.md; do
        [ -f "$unpacked/docs/$reference" ] || continue
        cp "$unpacked/docs/$reference" "$config_dir/$reference"
        refreshed=$((refreshed + 1))
    done
    written=0
    kept=0
    for theme in "$unpacked"/docs/themes/*.kdl; do
        [ -f "$theme" ] || continue
        if copy_if_absent "$theme" "$config_dir/themes/$(basename "$theme")"; then
            written=$((written + 1))
        else
            kept=$((kept + 1))
        fi
    done
    say ""
    say "Configuration in $config_dir"
    say "  $config_said; $written themes written, $kept left alone; $refreshed references refreshed"
fi

# ---- the desktop entry, for the build that opens a window ---------------

# Only the `gui` build, and only where there is a desktop to register with:
# a `.desktop` file is freedesktop's, so it means nothing on macOS or
# Windows, and a terminal-only build has no window to launch.
if [ "$DESKTOP" = 1 ] && [ "$BUILD" = gui ] && [ "$os_name" != macos ] \
   && [ -f "$unpacked/assets/maxgus.desktop" ]; then
    # Beside the binary when that `share` is one the desktop reads, and in
    # the user's own data directory otherwise. `~/bin` has a `~/share` beside
    # it that no desktop looks in, and an entry there was a stray directory
    # in the home and a menu with nothing new in it.
    data="${XDG_DATA_HOME:-$HOME/.local/share}"
    case "$PREFIX" in
        */bin)
            sibling="$(dirname "$PREFIX")/share"
            case ":${XDG_DATA_DIRS:-/usr/local/share:/usr/share}:" in
                *":$sibling:"*)
                    if [ -w "$(dirname "$PREFIX")" ]; then
                        data="$sibling"
                    fi ;;
            esac ;;
    esac
    apps="$data/applications"
    icons="$data/icons/hicolor/scalable/apps"
    if mkdir -p "$apps" "$icons" 2>/dev/null; then
        # A launcher does not see the shell's PATH, so the entry names the
        # binary in full — quoted, as the specification wants for a path with
        # a space in it, and escaped for sed.
        exec_path=$(printf '%s' "$PREFIX/maxgus" | sed -e 's/[\\"`$]/\\\\&/g')
        replacement=$(printf '%s' "\"$exec_path\"" | sed -e 's/[|&\\]/\\&/g')
        sed -e "s|^Exec=maxgus |Exec=$replacement |" \
            -e "s|^TryExec=maxgus\$|TryExec=$(printf '%s' "$PREFIX/maxgus" | sed -e 's/[|&\\]/\\&/g')|" \
            "$unpacked/assets/maxgus.desktop" > "$apps/maxgus.desktop"
        chmod 644 "$apps/maxgus.desktop"
        [ -f "$unpacked/assets/maxgus.svg" ] && cp "$unpacked/assets/maxgus.svg" "$icons/maxgus.svg"
        # Tell the desktop, where it wants telling. Neither is required and
        # neither failing matters: the entry is read at the next login.
        if command -v update-desktop-database >/dev/null 2>&1; then
            update-desktop-database "$apps" >/dev/null 2>&1 || true
        fi
        if command -v gtk-update-icon-cache >/dev/null 2>&1; then
            gtk-update-icon-cache -qtf "$data/icons/hicolor" >/dev/null 2>&1 || true
        fi
        say ""
        say "Listed in the application menu: $apps/maxgus.desktop"
    fi
fi

say ""
case ":$PATH:" in
    *":$PREFIX:"*)
        # On the path is not the same as first on it.
        found=$(command -v maxgus 2>/dev/null || true)
        if [ -n "$found" ] && [ "$found" != "$PREFIX/maxgus" ]; then
            say "Another maxgus, at $found, comes first on your PATH."
            say "Run this one in full, or take the other away:"
            say "    $PREFIX/maxgus FILE"
        else
            say "Run it: maxgus FILE"
        fi ;;
    *)
        say "$PREFIX is not on your PATH. Either run it in full:"
        say "    $PREFIX/maxgus FILE"
        say "or add $PREFIX to PATH in your shell's startup file." ;;
esac

if [ "$BUILD" = gui ]; then
    say ""
    say "This build opens a window. \`maxgus -nw FILE\` uses the terminal."
fi
say "Press C-h t inside it for a short guide."
