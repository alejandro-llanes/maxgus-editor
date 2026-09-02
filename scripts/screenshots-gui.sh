#!/usr/bin/env bash
#
# Takes the screenshots of the `gui` build that the website and the README
# use.
#
#     ./scripts/screenshots-gui.sh                 # all of them
#     ./scripts/screenshots-gui.sh magit tree      # only those
#     ./scripts/screenshots-gui.sh --list
#
# Each one is the editor itself, driven by keystrokes and photographed — not
# a mock-up, and not a capture of somebody's desktop with their wallpaper
# and their notifications in it.
#
# # Where it runs
#
# Inside `cage`, a compositor whose whole job is to run one program, on
# wlroots' headless backend. That is worth the trouble for three reasons:
#
#   - nothing appears on the screen of whoever runs it. There is no window
#     to cover, to move, or to type into by accident.
#   - the keystrokes go to that compositor rather than to whatever is
#     focused, so a shot cannot be spoiled by the person at the keyboard and
#     cannot spoil what they were doing.
#   - the window is exactly 1280x720 every time, so re-taking one picture
#     replaces it rather than producing something that has to be cropped to
#     match the others.
#
# Headless does not mean software-rendered: the editor still draws through
# the GPU, so the ligatures and the blur behind the popups are the real
# thing rather than a fallback.
#
# # What it needs
#
#     pacman -S cage grim wtype
#
# and a `gui` build, which `./scripts/build-variants.sh` leaves in
# `target/variants/maxgus-gui`.
#
# The editor is started with `scripts/screenshots-gui.kdl` rather than with
# your own configuration, so the pictures show what ships.
#
# The output is `docs/screenshots/gui-<name>.png`, which is where
# `site/index.html` and the README look for them and what
# `.github/workflows/pages.yml` publishes.
#
set -euo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
out="$root/docs/screenshots"
binary="$root/target/variants/maxgus-gui"
[ -x "$binary" ] || binary="$root/target/release/maxgus"

WIDTH=1280
HEIGHT=720

# A copy of this project, at a path with nobody's name in it.
#
# Every picture shows a path somewhere — the mode line, the echo area, the
# grep results, the terminal's prompt — and these go on a public page. A
# copy costs a second and keeps somebody's home directory out of all of
# them. It is named `maxgus-editor` so the file tree is rooted where a
# reader would expect.
PROJECT=""

project_copy() {
    local dir=/tmp/maxgus-screenshots/maxgus-editor
    if [ -d "$dir/.git" ]; then
        printf '%s' "$dir"
        return
    fi
    rm -rf "$dir"
    mkdir -p "$dir"
    # What is committed, so a working tree half way through something does
    # not end up in the pictures.
    git -C "$root" archive HEAD | tar -x -C "$dir"
    git -C "$dir" init -q -b main
    git -C "$dir" add -A
    git -C "$dir" \
        -c user.name=maxgus -c user.email=maxgus@example.com \
        -c commit.gpgsign=false \
        commit -q -m "A very small Emacs, written in Rust"
    printf '%s' "$dir"
}

say() { printf '\033[1m%s\033[0m\n' "$*"; }
die() { printf 'screenshots-gui: %s\n' "$*" >&2; exit 1; }

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
ctrl_shift() { wtype -M ctrl -M shift -k "$1" -m shift -m ctrl; }

# `grim` writes what the compositor has on its output, which is the editor
# and nothing else — no bar, no wallpaper, no cursor.
capture() {
    grim "$out/gui-$1.png"
    printf '  %-7s %s\n' "$(du -h "$out/gui-$1.png" | cut -f1)" "docs/screenshots/gui-$1.png"
}
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

# ---- the shots ----------------------------------------------------------
#
# One function per picture, each ending in `capture`. The pauses are part of
# the work rather than padding: the editor draws on a lull, so a shot taken
# without them is a shot of the editor mid-thought.

shot_editor() {
    panel_open
    pause 1.0
    capture editor
}

shot_which_key() {
    pause 0.6
    ctrl c                              # hold a prefix, and read the panel
    pause 1.8
    capture which-key
}

shot_buffers() {
    pause 0.6
    ctrl x; pause 0.3; key b            # the switcher, over blurred code
    pause 1.6
    capture buffers
}

shot_command() {
    pause 0.6
    alt x; pause 0.5
    text "buf"                          # M-x, fuzzy-matching as it is typed
    pause 1.6
    capture command
}

shot_grammar() {
    pause 0.6
    alt x; pause 0.4
    text "install-grammar"; pause 1.0
    key Return; pause 3.0               # every parser tree-sitter lists
    text "zig"; pause 2.0
    capture grammar
}

shot_ligatures() {
    pause 0.8
    ctrl s; text "=>"; pause 1.0        # where the operators are
    key Return; pause 0.6
    ctrl g; pause 1.2
    capture ligatures
}

# There is no shot of the beacon here. The light fades in a few hundred
# milliseconds and a still of it, caught at the right moment, reads as a
# smudge beside the cursor rather than as the thing it is.
# `docs/screenshots/beacon.svg` — drawn by `cargo run --example screenshot`,
# which composes the frame rather than photographing it — shows it properly.

shot_tree() {
    panel_open
    pause 0.8
    ctrl x; pause 0.25; key t; pause 0.25; key 1
    pause 0.6
    for _ in 1 2 3 4 5 6; do key Down; pause 0.2; done
    pause 1.0
    capture tree
}

shot_magit() {
    pause 0.8
    ctrl x; pause 0.3; key g            # the status view
    pause 2.0
    for _ in 1 2 3 4 5 6 7 8; do key Down; pause 0.2; done
    key Tab; pause 1.8                  # with one diff opened
    capture magit
}

shot_terminal() {
    pause 0.6
    ctrl x; pause 0.25; key t; pause 0.25; key v
    pause 2.5
    text "cargo --version"; key Return
    pause 2.0
    capture terminal
}

shot_grep() {
    pause 0.6
    meta s; pause 0.4; key g            # M-s g: search the project
    pause 0.8
    text "impl Highlighter"; key Return
    pause 3.0
    capture grep
}

shot_cursors() {
    pause 0.8
    ctrl s; text "score"; pause 0.6     # find a word, then take the next few
    key Return; pause 0.5
    ctrl g; pause 0.5
    for _ in 1 2 3; do ctrl_shift greater; pause 0.5; done
    pause 1.0
    capture cursors
}

shot_dired() {
    pause 0.6
    ctrl x; pause 0.3; key d
    pause 0.8
    key Return
    pause 2.0
    capture dired
}

shot_undo() {
    pause 0.6
    meta greater; pause 0.5             # at the end, where nothing is in the way
    key Return
    text "// one way of putting it"; pause 0.5
    ctrl underscore; pause 0.5          # undone,
    text "// and another"; pause 0.5    # then a different edit: a branch
    # By name rather than by `C-x U`: a virtual keyboard cannot produce a
    # distinct uppercase letter here — shift and `u` arrive as `C-x u`,
    # which is undo, which is a different picture entirely.
    alt x; pause 0.4
    text "undo-tree-visualize"; pause 0.8
    key Return
    pause 1.8
    capture undo
}

SHOTS="editor which-key buffers command grammar ligatures tree magit \
       terminal grep cursors dired undo"

# The side panel: the file tree, the outline and the buffer list, which is
# what most of these pictures should have beside the code.
panel_open() {
    pause 0.8
    ctrl x; pause 0.25; key t; pause 0.25; key t
    pause 1.4
}

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
            --config "$root/scripts/screenshots-gui.kdl" \
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

take() {
    local name=$1
    say "$name"
    log_dir=$(mktemp -d)

    # Most shots are of this project, which is real code with real symbols
    # and a real tree. magit is of a repository made for it.
    workdir="$PROJECT"
    opens="crates/maxgus-core/src/fuzzy.rs"
    if [ "$name" = magit ]; then
        workdir=$(mktemp -d /tmp/maxgus-shot-repo-XXXXXX)
        fixture_repo "$workdir"
        opens="src/lib.rs"
    fi

    start_compositor
    mkdir -p "$out"
    "shot_${name//-/_}"
    cleanup
    sleep 0.3

    [ -s "$out/gui-$name.png" ] || die "$name produced no picture; see $log_dir/compositor.log"
    rm -rf "$log_dir"
    case "$workdir" in /tmp/maxgus-shot-repo-*) rm -rf "$workdir" ;; esac
}

case "${1:-}" in
    --list) printf '%s\n' $SHOTS; exit 0 ;;
    -h|--help) sed -n '2,50p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
esac

for tool in cage grim wtype; do
    command -v "$tool" >/dev/null || die "$tool is not installed; see the top of this script"
done
[ -x "$binary" ] || die "no build at $binary — ./scripts/build-variants.sh"
"$binary" --version | grep -q gui || die "$binary is not a gui build"
export XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}"
PROJECT=$(project_copy)

for name in ${*:-$SHOTS}; do
    case " $SHOTS " in *" $name "*) ;; *) die "no shot called $name; --list shows them" ;; esac
    take "$name"
done
say "done — docs/screenshots"
