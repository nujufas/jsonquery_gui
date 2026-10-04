#!/usr/bin/env bash
# Tests of jsonquery-x11.sh. Nothing here starts jsonquery or touches the real
# home folder: each case gets a home of its own, and the program is a stand-in
# that writes the environment it was started with to a file.
#
#   scripts/test-jsonquery-x11.sh
set -uo pipefail

script="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)/jsonquery-x11.sh"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
fails=0 cases=0

pass() { cases=$((cases + 1)); }
fail() {
    cases=$((cases + 1))
    fails=$((fails + 1))
    printf 'FAIL: %s\n' "$1"
    shift
    [[ $# -eq 0 ]] || printf '      %s\n' "$@"
}
# expect <description> <command...>: the command must succeed.
expect() {
    local what="$1"
    shift
    if "$@" >/dev/null 2>&1; then pass; else fail "$what"; fi
}
# refuse <description> <command...>: the command must fail.
refuse() {
    local what="$1"
    shift
    if "$@" >/dev/null 2>&1; then fail "$what"; else pass; fi
}

# A new home, and the stand-in program `$stand_in`.
fresh() {
    home="$work/home-$cases-$RANDOM"
    mkdir -p "$home"
    log="$home/started-with.env"
    stand_in="$home/Programs/jsonquery_gui"
    mkdir -p "$(dirname "$stand_in")"
    printf '#!/bin/sh\nenv > %s\n' "'$log'" >"$stand_in"
    chmod +x "$stand_in"
    entry="$home/.local/share/applications/jsonquery-gui-x11.desktop"
    icon="$home/.local/share/icons/hicolor/128x128/apps/jsonquery-gui-x11.png"
}

# The script as run in a desktop of the given kind, in the current `$home`.
desk() {
    local kind="$1"
    shift
    local -a vars=(PATH="$PATH" HOME="$home")
    case "$kind" in
    wayland) vars+=(XDG_SESSION_TYPE=wayland WAYLAND_DISPLAY=wayland-0 DISPLAY=:0) ;;
    x11) vars+=(XDG_SESSION_TYPE=x11 DISPLAY=:0) ;;
    wayland-no-x) vars+=(XDG_SESSION_TYPE=wayland WAYLAND_DISPLAY=wayland-0) ;;
    esac
    env -i "${vars[@]}" bash "$script" "$@"
}

exec_line() { grep '^Exec=' "$1"; }

# --- the entry --------------------------------------------------------------

fresh
out="$(desk wayland install "$stand_in" 2>&1)"
expect "install exits 0" test $? -eq 0
expect "install writes the entry" test -f "$entry"
expect "install writes the icon, a PNG" bash -c "head -c 8 '$icon' | od -An -tx1 | grep -q '89 50 4e 47 0d 0a 1a 0a'"
expect "the entry runs the program without the Wayland variables" \
    grep -qxF "Exec=env -u WAYLAND_DISPLAY -u WAYLAND_SOCKET \"$stand_in\"" "$entry"
expect "the entry names itself jsonquery (X11)" grep -qxF 'Name=jsonquery (X11)' "$entry"
expect "the entry has no StartupWMClass (it would claim every jsonquery window)" \
    bash -c "! grep -q StartupWMClass '$entry'"
expect "the entry points at the icon it wrote" grep -qxF "Icon=$icon" "$entry"
expect "the answer says Created and where" grep -q "^Created .*$entry" <<<"$out"
if command -v desktop-file-validate >/dev/null; then
    expect "the entry passes desktop-file-validate" desktop-file-validate "$entry"
fi

# Running the entry, as the desktop does, starts the program without Wayland.
if command -v gio >/dev/null; then
    env -i PATH="$PATH" HOME="$home" WAYLAND_DISPLAY=wayland-0 WAYLAND_SOCKET=3 DISPLAY=:0 \
        gio launch "$entry" >/dev/null 2>&1
    for _ in $(seq 20); do
        [[ -s "$log" ]] && break
        sleep 0.1
    done
    if [[ -s "$log" ]]; then
        expect "started from the entry: no WAYLAND_DISPLAY" bash -c "! grep -q '^WAYLAND_DISPLAY=' '$log'"
        expect "started from the entry: no WAYLAND_SOCKET" bash -c "! grep -q '^WAYLAND_SOCKET=' '$log'"
        expect "started from the entry: DISPLAY is kept" grep -qxF 'DISPLAY=:0' "$log"
    else
        fail "gio launch did not start the stand-in program from the entry"
    fi
fi

# --- again, and a different program ---------------------------------------

other="$home/Moved Programs/jsonquery_gui"
mkdir -p "$(dirname "$other")"
cp "$stand_in" "$other"
out="$(desk wayland install "$other" 2>&1)"
expect "the second install says Replaced" grep -q '^Replaced ' <<<"$out"
expect "and the entry now runs the program that has a space in its path" \
    grep -qxF "Exec=env -u WAYLAND_DISPLAY -u WAYLAND_SOCKET \"$other\"" "$entry"
expect "and leaves no temporary files behind" \
    bash -c "[ \"\$(ls -A '$(dirname "$entry")' | wc -l)\" -eq 1 ]"

# A read-only file, and a symlink, are replaced as they stand.
chmod 444 "$entry"
expect "a read-only entry is replaced" desk wayland install "$stand_in"
expect "... with the new text" grep -qxF "Exec=env -u WAYLAND_DISPLAY -u WAYLAND_SOCKET \"$stand_in\"" "$entry"
rm -f "$entry"
echo 'elsewhere' >"$home/elsewhere"
ln -s "$home/elsewhere" "$entry"
expect "a symlink in its place is replaced" desk wayland install "$stand_in"
expect "... by a file, and what it pointed at is untouched" \
    bash -c "[ ! -L '$entry' ] && [ \"\$(cat '$home/elsewhere')\" = elsewhere ]"

# --- which desktop ----------------------------------------------------------

fresh
out="$(desk x11 install "$stand_in" 2>&1)"
expect "not on Wayland: exits 0" test $? -eq 0
expect "not on Wayland: says there is nothing to set up" grep -q 'Nothing to set up' <<<"$out"
expect "not on Wayland: writes nothing" test ! -e "$entry"
expect "--force writes it all the same" desk x11 install --force "$stand_in"
expect "... the entry" test -f "$entry"

fresh
out="$(desk wayland-no-x install "$stand_in" 2>&1)"
expect "Wayland without XWayland: says X11 is not available" grep -q 'no X11 support' <<<"$out"
expect "Wayland without XWayland: writes nothing" test ! -e "$entry"
refuse "Wayland without XWayland: run fails" desk wayland-no-x run "$stand_in"
out="$(desk wayland check "$stand_in" 2>&1)"
expect "check on Wayland says what was found" grep -q "jsonquery found:  $stand_in" <<<"$out"
expect "check on Wayland names the verdict" grep -q 'Wayland with XWayland' <<<"$out"
expect "check on a desktop that is not Wayland says so" \
    bash -c "env -i PATH='$PATH' HOME='$home' XDG_SESSION_TYPE=x11 DISPLAY=:0 bash '$script' check '$stand_in' | grep -q 'not Wayland'"

# --- run --------------------------------------------------------------------

fresh
desk wayland run "$stand_in" >/dev/null 2>&1
expect "run starts the program" test -s "$log"
expect "run: no WAYLAND_DISPLAY" bash -c "! grep -q '^WAYLAND_DISPLAY=' '$log'"
expect "run: DISPLAY is kept" grep -qxF 'DISPLAY=:0' "$log"
expect "run writes no entry" test ! -e "$entry"

# --- finding the program ----------------------------------------------------

fresh
bundle="$home/unpacked"
mkdir -p "$bundle"
cp "$script" "$bundle/"
cp "$stand_in" "$bundle/jsonquery_gui"
env -i PATH="$PATH" HOME="$home" XDG_SESSION_TYPE=wayland DISPLAY=:0 bash "$bundle/jsonquery-x11.sh" install >/dev/null 2>&1
expect "the program next to the script is found" grep -qxF "Exec=env -u WAYLAND_DISPLAY -u WAYLAND_SOCKET \"$bundle/jsonquery_gui\"" "$entry"

fresh
bundle="$home/downloads"
mkdir -p "$bundle"
cp "$script" "$bundle/"
cp "$stand_in" "$bundle/jsonquery_gui-0.5.0-x86_64.AppImage"
env -i PATH="$PATH" HOME="$home" XDG_SESSION_TYPE=wayland DISPLAY=:0 bash "$bundle/jsonquery-x11.sh" install >/dev/null 2>&1
expect "an AppImage next to the script is found" grep -q 'jsonquery_gui-0.5.0-x86_64.AppImage"$' "$entry"
cp "$stand_in" "$bundle/jsonquery_gui-0.4.2-x86_64.AppImage"
rm -f "$entry"
refuse "two AppImages next to the script: it asks which" \
    env -i PATH="$PATH" HOME="$home" XDG_SESSION_TYPE=wayland DISPLAY=:0 bash "$bundle/jsonquery-x11.sh" install
expect "... and writes nothing" test ! -e "$entry"
expect "... until one is named" \
    env -i PATH="$PATH" HOME="$home" XDG_SESSION_TYPE=wayland DISPLAY=:0 bash "$bundle/jsonquery-x11.sh" install "$bundle/jsonquery_gui-0.4.2-x86_64.AppImage"

fresh
bin="$home/bin"
mkdir -p "$bin"
cp "$stand_in" "$bin/jsonquery-gui"
env -i PATH="$bin:$PATH" HOME="$home" XDG_SESSION_TYPE=wayland DISPLAY=:0 bash "$script" install >/dev/null 2>&1
expect "a jsonquery-gui on the PATH (the snap's name) is found" \
    grep -qxF "Exec=env -u WAYLAND_DISPLAY -u WAYLAND_SOCKET \"$bin/jsonquery-gui\"" "$entry"

# A link keeps its own path: /snap/bin/jsonquery-gui is one, to a program that looks at its name.
fresh
ln -s "$stand_in" "$home/jsonquery-gui"
desk wayland install "$home/jsonquery-gui" >/dev/null 2>&1
expect "a symlink to the program is not followed" \
    grep -qxF "Exec=env -u WAYLAND_DISPLAY -u WAYLAND_SOCKET \"$home/jsonquery-gui\"" "$entry"

fresh
printf '#!/bin/sh\n' >"$home/not-executable"
refuse "a file that cannot be run is refused" desk wayland install "$home/not-executable"
refuse "a missing file is refused" desk wayland install "$home/missing"
expect "... and nothing is written" test ! -e "$entry"

fresh
# shellcheck disable=SC2016 # the dollar and the tick are meant literally
for odd in 'with"quote' 'with$dollar' 'with%percent' 'with\backslash' 'with`tick'; do
    mkdir -p "$home/$odd"
    cp "$stand_in" "$home/$odd/jsonquery_gui"
    refuse "a path with $odd is refused" desk wayland install "$home/$odd/jsonquery_gui"
done
expect "... and nothing is written" test ! -e "$entry"

# --- the Desktop shortcut ---------------------------------------------------

fresh
mkdir -p "$home/Desktop"
desk wayland install --desktop "$stand_in" >/dev/null 2>&1
shortcut="$home/Desktop/jsonquery-gui-x11.desktop"
expect "--desktop puts a shortcut on the Desktop" test -f "$shortcut"
expect "... that can be run" test -x "$shortcut"
expect "... and is the same entry" cmp -s "$entry" "$shortcut"

fresh
out="$(desk wayland install --desktop "$stand_in" 2>&1)"
expect "--desktop with no Desktop folder still exits 0" test $? -eq 0
expect "... says there is none" grep -q 'no Desktop folder' <<<"$out"
expect "... and the menu entry is there" test -f "$entry"

# --- remove -----------------------------------------------------------------

fresh
mkdir -p "$home/Desktop"
desk wayland install --desktop "$stand_in" >/dev/null 2>&1
out="$(desk wayland remove 2>&1)"
expect "remove takes the menu entry away" test ! -e "$entry"
expect "... and the icon" test ! -e "$icon"
expect "... and the Desktop shortcut" test ! -e "$home/Desktop/jsonquery-gui-x11.desktop"
expect "... and says what it removed" grep -q '^Removed ' <<<"$out"
out="$(desk wayland remove 2>&1)"
expect "remove again exits 0" test $? -eq 0
expect "... saying there was nothing to remove" grep -q 'Nothing to remove' <<<"$out"

# --- the command line -------------------------------------------------------

expect "--help exits 0" bash "$script" --help
refuse "an unknown option is refused" bash "$script" --nonsense

printf '%d cases, %d failed\n' "$cases" "$fails"
[[ "$fails" -eq 0 ]]
