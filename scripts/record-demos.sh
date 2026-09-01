#!/usr/bin/env bash
#
# Records the short clips the website plays, from the real `gui` build.
#
#     ./scripts/record-demos.sh                 # all of them
#     ./scripts/record-demos.sh blur treefile   # only those
#     ./scripts/record-demos.sh --list
#
# Each clip is the editor itself being typed at — not a mock-up, and not a
# capture of somebody's desktop with their wallpaper and their notifications
# in it.
#
# # Where it runs
#
# Inside `cage`, a compositor whose whole job is to run one program, on
# wlroots' headless backend. That is worth the trouble for three reasons:
#
#   - nothing appears on the screen of whoever runs it. There is no window
#     to cover, to move, or to type into by accident.
#   - the keystrokes go to that compositor rather than to whatever is
#     focused, so a take cannot be spoiled by the person at the keyboard and
#     cannot spoil what they were doing.
#   - the output is exactly 1280x720 every time, so re-recording one clip
#     replaces it rather than producing something that has to be cropped to
#     match the others.
#
# Headless does not mean software-rendered: the editor still draws through
# the GPU, so the ligatures and the blur behind the popups are the real
# thing rather than a fallback.
#
# # What it needs
#
#     pacman -S cage wf-recorder wtype ffmpeg
#
# and a `gui` build, which `./scripts/build-variants.sh` leaves in
# `target/variants/maxgus-gui`.
#
# The editor is started with `scripts/record-demos.kdl` rather than with your
# own configuration, so the clips show what ships.
#
# The output is `docs/media/<name>.webm`, `.mp4` and `.jpg` — VP9 for
# browsers that take it, H.264 for the rest, and a poster frame to show
# before either has loaded. `site/index.html` plays them and
# `.github/workflows/pages.yml` ships them.
#
# The raw captures are kept in `target/demo-raw`, so the encoding can be
# changed and everything re-encoded without recording anything again:
#
#     ./scripts/record-demos.sh --encode-only
#
set -euo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
out="$root/docs/media"
raw_dir="$root/target/demo-raw"
binary="$root/target/variants/maxgus-gui"
[ -x "$binary" ] || binary="$root/target/release/maxgus"

WIDTH=1280
HEIGHT=720
FPS=30

# The demo project is this one: real code, real git history, real symbols.
PROJECT="$root"

say() { printf '\033[1m%s\033[0m\n' "$*"; }
die() { printf 'record-demos: %s\n' "$*" >&2; exit 1; }

# ---- pressing keys -------------------------------------------------------
#
# `wtype` talks to a compositor rather than to a window, so everything here
# goes to the nested one through `WAYLAND_DISPLAY` and nothing reaches the
# desktop this was started from.

key()      { wtype -k "$1"; }
ctrl()     { wtype -M ctrl -k "$1" -m ctrl; }
alt()      { wtype -M alt -k "$1" -m alt; }
# `M-<something>`. The keysym is what the editor sees, so a `<` is `less`
# rather than shift and a comma: sending the modifier and the unshifted key
# gives `M-,`, which is a different command and was quietly being recorded
# as one.
meta()     { wtype -M alt -k "$1" -m alt; }
text()     { wtype "$1"; }

# The editor draws on a pause. A take with no pauses in it records the
# editor mid-thought, so these are part of the clip rather than padding.
pause()    { sleep "$1"; }

# `C-x C-f` with the path typed in, which is how a person opens a file and
# therefore what a clip of the editor should show.
open_file() {
    ctrl x; ctrl f
    pause 0.5
    # The prompt opens pre-filled with the directory it would look in, so
    # a path typed straight in would be appended to it rather than replace
    # it. `C-a C-k` is how a person clears it, and what the minibuffer's own
    # keymap binds.
    ctrl a; ctrl k
    pause 0.2
    text "$PROJECT/$1"
    key Return
    pause 1.0
}

# ---- the clips -----------------------------------------------------------

clip_scrolling() {
    open_file "crates/maxgus-core/src/editor.rs"
    pause 1.0
    for _ in 1 2 3 4 5; do ctrl v; pause 0.45; done
    for _ in 1 2 3; do alt v; pause 0.45; done
    meta greater; pause 1.4          # M-> : the far end of a long file
    meta less;  pause 1.2          # M-< : and back to the top
}

clip_blur() {
    pause 1.0
    ctrl x; key b                       # the buffer list, over the code
    pause 1.8
    ctrl g; pause 0.6
    alt x                               # M-x, and the same blur under it
    pause 0.4
    text "describe"
    pause 1.8
    ctrl g; pause 0.8
}

clip_cursor() {
    pause 1.0
    meta greater; pause 1.5          # a jump, and the light after it
    meta less;  pause 1.5
    ctrl s; text "pub fn"; pause 0.8    # and another, out of a search
    key Return; pause 1.5
}

clip_treefile() {
    pause 1.0
    # `C-x t t` opens the side panel and `C-x t 1` moves into it — opening
    # it does not take the cursor there, which is right for editing and
    # wrong for a clip that then presses Down.
    ctrl x; pause 0.25; key t; pause 0.25; key t
    pause 1.8
    ctrl x; pause 0.25; key t; pause 0.25; key 1
    pause 1.0
    for _ in 1 2 3 4 5 6; do key Down; pause 0.3; done
    key Return; pause 2.2                # open what is under the cursor
}

clip_magit() {
    pause 1.0
    ctrl x; key g                       # the status view
    pause 2.2
    # Down to the modified file rather than to an untracked one: an
    # untracked file has no diff to open, and a clip of nothing happening
    # is not a clip of magit.
    for _ in 1 2 3 4 5 6 7 8; do key Down; pause 0.22; done
    key Tab; pause 2.2                  # open its diff
    text "s"; pause 2.2                 # stage it, and watch it move
}

clip_whichkey() {
    pause 1.0
    ctrl c; pause 2.4                   # hold the prefix, read the panel
    ctrl g; pause 0.5
    ctrl x; pause 2.4
    ctrl g; pause 0.8
}

clip_ligatures() {
    pause 1.2
    ctrl s; text "=>"; pause 1.0        # where the operators are
    key Return; pause 1.0
    ctrl g; pause 2.6
}

clip_grammar() {
    pause 1.0
    alt x; pause 0.4
    text "install-grammar"; pause 1.2
    key Return; pause 3.0               # the menu of every parser there is
    text "zig"; pause 2.2
    ctrl g; pause 0.8
}

CLIPS="scrolling blur cursor treefile magit whichkey ligatures grammar"

# ---- a repository to show ------------------------------------------------

# magit shows a working tree, and the working tree here is whoever is
# recording's own — half-finished, full of the very files that make the clip,
# and different every time. So that one clip gets a repository built for it:
# two commits, a couple of edits and an untracked file, which is what a status
# view is for and looks the same in every take.
fixture_repo() {
    local dir=$1
    rm -rf "$dir"
    mkdir -p "$dir/src"
    cat > "$dir/README.md" <<'EOF'
# ledger

A very small double-entry ledger, for the money you have already spent.

    ledger add "coffee" 3.40
    ledger balance
EOF
    cat > "$dir/src/main.rs" <<'EOF'
//! Reads the ledger, prints what is in it.

fn main() {
    let entries = ledger::read("ledger.txt").unwrap_or_default();
    for entry in &entries {
        println!("{:>8.2}  {}", entry.amount, entry.what);
    }
    println!("{:>8.2}  total", ledger::total(&entries));
}
EOF
    cat > "$dir/src/lib.rs" <<'EOF'
//! An entry is an amount and what it was for.

pub struct Entry {
    pub amount: f64,
    pub what: String,
}

pub fn total(entries: &[Entry]) -> f64 {
    entries.iter().map(|e| e.amount).sum()
}
EOF
    git -C "$dir" init -q -b main
    git -C "$dir" add -A
    git -C "$dir" \
        -c user.name=maxgus -c user.email=maxgus@example.com \
        -c commit.gpgsign=false \
        commit -q -m "Read a ledger and print what is in it"

    # Something to look at in the status view: two files changed, one new.
    cat >> "$dir/src/lib.rs" <<'EOF'

/// The largest single entry, which is usually the surprising one.
pub fn biggest(entries: &[Entry]) -> Option<&Entry> {
    entries.iter().max_by(|a, b| a.amount.total_cmp(&b.amount))
}
EOF
    printf '\n    ledger biggest\n' >> "$dir/README.md"
    printf 'ledger.txt\ntarget/\n' > "$dir/.gitignore"
    printf '%s\n' '# What to do next' '' '- group by month' '- read from stdin' \
        > "$dir/NOTES.md"
}

# ---- the compositor -----------------------------------------------------

# Everything started for one take, so a failure half way through takes its
# processes with it rather than leaving a compositor running for ever.
compositor=
recorder=
cleanup() {
    [ -n "$recorder"   ] && kill "$recorder"   2>/dev/null || true
    # By pid, and only ever by pid: a pattern that matched "cage" or a
    # compositor's name could match the session the person running this is
    # sitting in.
    [ -n "$compositor" ] && kill "$compositor" 2>/dev/null || true
    recorder= compositor=
    unset WAYLAND_DISPLAY
}
trap cleanup EXIT INT TERM

start_compositor() {
    local before after new
    # The sockets that existed before, so the one this starts can be picked
    # out by name rather than by being the newest. A stray compositor left
    # over from something else must not be the one that gets recorded.
    before=$(ls "$XDG_RUNTIME_DIR" | grep '^wayland-[0-9]*$' | sort)

    # `WAYLAND_DISPLAY` unset, or wlroots would nest a window in the session
    # this was started from instead of going headless.
    # Started *in* the directory the take is about: git resolves the top of
    # a working tree from where it is run, so a magit clip recorded from
    # somewhere else shows that somewhere else.
    # `exec`, so the pid recorded below is the compositor itself rather than
    # a shell that happens to have started one — killing the shell would
    # leave cage running, and a leaked compositor holds a socket that the
    # next take could pick up by mistake.
    ( cd "$workdir" || exit 1
      exec env -u WAYLAND_DISPLAY -u HYPRLAND_INSTANCE_SIGNATURE \
        WLR_BACKENDS=headless \
        WLR_LIBINPUT_NO_DEVICES=1 \
        cage -- "$binary" --gui \
            --config "$root/scripts/record-demos.kdl" \
            --directory "$workdir" \
            "$workdir/$opens" \
        >"$log_dir/compositor.log" 2>&1 ) &
    compositor=$!

    for _ in $(seq 1 80); do
        after=$(ls "$XDG_RUNTIME_DIR" | grep '^wayland-[0-9]*$' | sort)
        new=$(comm -13 <(printf '%s\n' "$before") <(printf '%s\n' "$after") | head -1)
        [ -n "$new" ] && break
        sleep 0.25
    done
    [ -n "$new" ] || die "cage did not start; see $log_dir/compositor.log"
    export WAYLAND_DISPLAY="$new"

    # The editor has to have drawn before anything is typed at it, or the
    # first keystrokes land in a window that is not there yet.
    sleep 3

    # And the first keystroke after that is swallowed regardless: `wtype`
    # makes a virtual keyboard per invocation, and the earliest one is gone
    # before the editor is listening. Down and Up absorb it and cancel each
    # other out. `C-g` would do as well except that it leaves "Quit" in the
    # echo area, and the echo area is on screen for the whole clip.
    key Down; sleep 0.35
    key Up;   sleep 0.35
    key Down; sleep 0.35
    key Up;   sleep 0.6
}

record() {
    local name=$1 raw
    say "recording $name"

    # Most clips are of this project, which is real code with real symbols
    # and a real tree. magit is of a repository made for it.
    workdir="$PROJECT"
    opens="crates/maxgus-core/src/fuzzy.rs"
    if [ "$name" = magit ]; then
        workdir=$(mktemp -d /tmp/maxgus-demo-repo-XXXXXX)
        fixture_repo "$workdir"
        opens="src/lib.rs"
    fi
    log_dir=$(mktemp -d)
    mkdir -p "$raw_dir"
    raw="$raw_dir/$name.mkv"

    start_compositor

    wf-recorder -f "$raw" -r "$FPS" --codec libx264 -x yuv420p \
        >"$log_dir/recorder.log" 2>&1 &
    recorder=$!
    sleep 1.5

    "clip_$name"

    # SIGINT, not SIGKILL: wf-recorder writes the file's index on its way
    # out, and a killed recording is one no player will seek in.
    kill -INT "$recorder" 2>/dev/null || true
    for _ in $(seq 1 40); do kill -0 "$recorder" 2>/dev/null || break; sleep 0.25; done
    recorder=
    cleanup
    sleep 0.5

    [ -s "$raw" ] || die "$name recorded nothing; see $log_dir/recorder.log"
    encode "$raw" "$name"
    rm -rf "$log_dir"
    case "$workdir" in /tmp/maxgus-demo-repo-*) rm -rf "$workdir" ;; esac
}

# H.264 and VP9, both silent: there is nothing to hear, and a track a
# browser considers muted is what lets it play without being asked.
#
# H.264 first, and it is what nearly everyone will fetch. That is the reverse
# of the usual advice, and it is what measuring said: the capture is already
# H.264, so one transcode of it is smaller at the same quality than anything
# VP9 or AV1 can do from the same source — 0.88 MB against 1.26 for the
# worst clip here. The VP9 is kept for a browser built without H.264, which
# on Linux is a real if uncommon thing.
encode() {
    local raw=$1 name=$2
    mkdir -p "$out"
    ffmpeg -y -loglevel error -i "$raw" \
        -vf "scale=$WIDTH:$HEIGHT:flags=lanczos" -an \
        -c:v libx264 -preset slow -crf 26 -pix_fmt yuv420p -movflags +faststart \
        "$out/$name.mp4"
    ffmpeg -y -loglevel error -i "$raw" \
        -vf "scale=$WIDTH:$HEIGHT:flags=lanczos" -an \
        -c:v libvpx-vp9 -crf 42 -b:v 0 -row-mt 1 -tile-columns 2 \
        -auto-alt-ref 1 -lag-in-frames 25 -deadline good -cpu-used 1 \
        "$out/$name.webm"
    # A frame with something on it rather than the first one, which is the
    # editor before it has been touched.
    ffmpeg -y -loglevel error -ss 2.5 -i "$raw" -frames:v 1 \
        -vf "scale=$WIDTH:$HEIGHT:flags=lanczos" -q:v 5 "$out/$name.jpg"
    ls -lh "$out/$name.mp4" "$out/$name.webm" "$out/$name.jpg" |
        awk '{ printf "  %-6s %s\n", $5, $9 }'
}

case "${1:-}" in
    --list) printf '%s\n' $CLIPS; exit 0 ;;
    --encode-only)
        shift
        for name in ${*:-$CLIPS}; do
            [ -s "$raw_dir/$name.mkv" ] || die "no capture kept for $name"
            say "encoding $name"
            encode "$raw_dir/$name.mkv" "$name"
        done
        exit 0 ;;
    -h|--help) sed -n '2,40p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
esac

for tool in cage wf-recorder wtype ffmpeg; do
    command -v "$tool" >/dev/null || die "$tool is not installed; see the top of this script"
done
[ -x "$binary" ] || die "no build at $binary — ./scripts/build-variants.sh"
"$binary" --version | grep -q gui || die "$binary is not a gui build"
export XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}"

for name in ${*:-$CLIPS}; do
    case " $CLIPS " in *" $name "*) ;; *) die "no clip called $name; --list shows them" ;; esac
    record "$name"
done
say "done — docs/media"
