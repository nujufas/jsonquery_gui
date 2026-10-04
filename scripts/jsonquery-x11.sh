#!/usr/bin/env bash
# jsonquery-x11.sh -- start jsonquery under X11, and put a "jsonquery (X11)"
# entry in the application menu (and, if asked, on the Desktop) that does it
# with a click.
#
# Why: on a Wayland desktop (the default on current Ubuntu, Fedora and others)
# jsonquery cannot take a file dropped on its window: the windowing library it is
# built on (winit) has no drag-and-drop on native Wayland. Run under X11 -- which
# every Wayland desktop offers to programs that ask for it, as XWayland -- and
# dropping files works. The one thing this changes is what the program is told
# about its display: the entry starts it with WAYLAND_DISPLAY and WAYLAND_SOCKET
# removed. Everything else is the program you already have.
#
# Works for the tar.gz, the AppImage, the snap and the Arch package alike. It
# runs in your own shell, so it can write your menu even where the snap itself
# cannot.
#
#   jsonquery-x11.sh                 add "jsonquery (X11)" to the application menu
#   jsonquery-x11.sh --desktop       ... and a shortcut on the Desktop too
#   jsonquery-x11.sh run             start jsonquery under X11 now, no shortcut
#   jsonquery-x11.sh remove          take the entry and the shortcut away again
#   jsonquery-x11.sh check           say what this desktop needs, and what was found
#
# The program is found by itself: the one next to this script (the tar.gz's
# jsonquery_gui, or the AppImage), else `jsonquery_gui` / `jsonquery-gui` on the
# PATH, else the snap. To name it, put its path last:
#
#   jsonquery-x11.sh ~/Apps/jsonquery_gui-0.5.0-x86_64.AppImage
#
# Run it again after moving or updating the program: the entry is rewritten.
# On a desktop that is not Wayland it does nothing, unless given --force.
set -euo pipefail

ID=jsonquery-gui-x11
DATA_HOME="${XDG_DATA_HOME:-$HOME/.local/share}"
ENTRY="$DATA_HOME/applications/$ID.desktop"
ICON="$DATA_HOME/icons/hicolor/128x128/apps/$ID.png"

say() { printf '%s\n' "$*"; }
die() {
    printf 'jsonquery-x11: %s\n' "$*" >&2
    exit 1
}

usage() {
    # The comment at the top of this file is the help.
    sed -n '2,/^set -euo pipefail/{/^set -euo/!s/^# \{0,1\}//p;}' "${BASH_SOURCE[0]}"
}

session_is_wayland() {
    [[ "${XDG_SESSION_TYPE:-}" == wayland || -n "${WAYLAND_DISPLAY:-}" ]]
}

# A path is made absolute without following the program's own symlink: the
# snap's /snap/bin/jsonquery-gui is one, to a program that looks at its name.
absolute() {
    local dir
    dir="$(cd -- "$(dirname -- "$1")" && pwd)"
    printf '%s/%s\n' "${dir%/}" "$(basename -- "$1")"
}

# What a menu entry cannot hold safely in a path: it is quoted, and these are
# the characters that quoting does not settle.
check_path() {
    case "$1" in
    *[\"\`\$\\%]* | *$'\n'* | *$'\t'*)
        die "the path '$1' has a character that a menu entry cannot hold safely (one of \" \` \$ \\ % or a tab); put the program somewhere with a plainer path and run this again"
        ;;
    esac
}

find_program() {
    local given="${1:-}" here candidate
    local -a images
    if [[ -n "$given" ]]; then
        [[ -f "$given" && -x "$given" ]] ||
            die "'$given' is not a program I can run (an AppImage must be made executable first: chmod +x '$given')"
        absolute "$given"
        return
    fi
    here="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
    if [[ -f "$here/jsonquery_gui" && -x "$here/jsonquery_gui" ]]; then
        printf '%s\n' "$here/jsonquery_gui"
        return
    fi
    shopt -s nullglob
    images=("$here"/jsonquery_gui*.AppImage)
    shopt -u nullglob
    if ((${#images[@]} == 1)) && [[ -x "${images[0]}" ]]; then
        printf '%s\n' "${images[0]}"
        return
    fi
    if ((${#images[@]} > 1)); then
        die "there is more than one AppImage next to this script (${images[*]##*/}); name the one to use, last on the command line"
    fi
    for candidate in jsonquery_gui jsonquery-gui; do
        if command -v "$candidate" >/dev/null 2>&1; then
            absolute "$(command -v "$candidate")"
            return
        fi
    done
    if [[ -x /snap/bin/jsonquery-gui ]]; then
        printf '%s\n' /snap/bin/jsonquery-gui
        return
    fi
    die "I could not find jsonquery. Name it, last on the command line: jsonquery-x11.sh /path/to/jsonquery_gui (an AppImage must be made executable first: chmod +x file)"
}

# Wayland and a way to X11 (XWayland): 0 = yes, 1 = not Wayland, 2 = Wayland
# with no X display to fall back on.
desktop_state() {
    session_is_wayland || return 1
    [[ -n "${DISPLAY:-}" ]] || return 2
    return 0
}

# Says why this desktop needs nothing (or cannot have it) and returns 1, unless
# --force was given.
need_x11_or_say() {
    local state=0
    desktop_state || state=$?
    case "$state" in
    0) return 0 ;;
    1) say "This is not a Wayland desktop (session type: ${XDG_SESSION_TYPE:-unknown}), so files can already be dropped on jsonquery's window. Nothing to set up. To add the entry anyway, run this again with --force." ;;
    2) say "This is a Wayland desktop with no X11 support (DISPLAY is not set), so jsonquery cannot be started under X11 here. If your desktop has an XWayland setting, turn it on and log in again. To add the entry anyway, run this again with --force." ;;
    esac
    ((force)) && return 0
    return 1
}

# Writes stdin to $1 with mode $2, replacing whatever is there (a symlink or a
# read-only file too): the whole file appears at once, or not at all.
write_file() {
    local dst="$1" mode="$2" tmp
    mkdir -p -- "$(dirname -- "$dst")"
    tmp="$(mktemp -- "$dst.XXXXXX")"
    if cat >"$tmp" && chmod "$mode" "$tmp" && mv -f -- "$tmp" "$dst"; then
        return 0
    fi
    rm -f -- "$tmp"
    die "could not write $dst"
}

entry_text() {
    cat <<EOF
[Desktop Entry]
# Written by jsonquery-x11.sh; running it again rewrites this file.
Type=Application
Name=jsonquery (X11)
GenericName=JSON Query Tool
Comment=jsonquery started under X11 (XWayland), so that dropping files on its window works on a Wayland desktop
Exec=env -u WAYLAND_DISPLAY -u WAYLAND_SOCKET "$1"
Icon=$ICON
Categories=Development;Utility;
Terminal=false
EOF
}

desktop_shortcut_path() {
    local dir
    dir="$(xdg-user-dir DESKTOP 2>/dev/null || true)"
    [[ -n "$dir" ]] || dir="$HOME/Desktop"
    # xdg-user-dir answers $HOME when the user has no Desktop folder.
    [[ "${dir%/}" != "${HOME%/}" ]] || return 1
    printf '%s/%s.desktop\n' "${dir%/}" "$ID"
}

cmd_install() {
    local program verb=Created shortcut=
    need_x11_or_say || return 0
    program="$(find_program "$program_arg")"
    check_path "$program"
    check_path "$ICON"
    [[ -e "$ENTRY" || -L "$ENTRY" ]] && verb=Replaced

    icon_base64 | base64 -d | write_file "$ICON" 644
    entry_text "$program" | write_file "$ENTRY" 644
    say "$verb the menu entry \"jsonquery (X11)\": $ENTRY"
    say "It starts: $program"

    if ((desktop)); then
        if shortcut="$(desktop_shortcut_path)" && [[ -d "$(dirname -- "$shortcut")" ]]; then
            entry_text "$program" | write_file "$shortcut" 755
            # GNOME shows a desktop file as launchable only once it is trusted.
            gio set "$shortcut" metadata::trusted true >/dev/null 2>&1 ||
                say "If the Desktop icon asks for it, right-click it and choose \"Allow Launching\"."
            say "Put a shortcut on the Desktop: $shortcut"
        else
            say "There is no Desktop folder to put a shortcut in; the menu entry is there all the same."
        fi
    fi

    say
    say "To use it: open the application menu (press the Super key), type jsonquery and"
    say "click \"jsonquery (X11)\". Drop files on its window as you would anywhere else."
    say "Moved or updated the program? Run this script again. To undo it, run it with the word remove after it."
}

cmd_remove() {
    local file removed=0
    for file in "$ENTRY" "$ICON" "$(desktop_shortcut_path || true)"; do
        if [[ -n "$file" && (-e "$file" || -L "$file") ]]; then
            rm -f -- "$file"
            say "Removed $file"
            removed=1
        fi
    done
    ((removed)) || say "Nothing to remove: there is no jsonquery (X11) entry."
}

cmd_run() {
    local program
    [[ -n "${DISPLAY:-}" ]] || die "there is no X display (DISPLAY is not set), so jsonquery cannot be started under X11 here"
    program="$(find_program "$program_arg")"
    exec env -u WAYLAND_DISPLAY -u WAYLAND_SOCKET "$program"
}

cmd_check() {
    local state=0 program
    desktop_state || state=$?
    say "Session type:     ${XDG_SESSION_TYPE:-unknown}"
    case "$state" in
    0) say "Verdict:          Wayland with XWayland: files cannot be dropped on jsonquery's window as it is; started under X11 they can." ;;
    1) say "Verdict:          not Wayland: dropping files already works, nothing to set up." ;;
    2) say "Verdict:          Wayland without XWayland (DISPLAY is not set): X11 mode is not available here." ;;
    esac
    if program="$(find_program "$program_arg" 2>&1)"; then
        say "jsonquery found:  $program"
    else
        say "jsonquery found:  no (${program#jsonquery-x11: })"
    fi
    if [[ -e "$ENTRY" ]]; then
        say "Menu entry:       $ENTRY"
    else
        say "Menu entry:       none yet"
    fi
}

command=install
case "${1:-}" in
install | run | remove | check)
    command="$1"
    shift
    ;;
esac
desktop=0
force=0
program_arg=
for arg in "$@"; do
    case "$arg" in
    -h | --help)
        usage
        exit 0
        ;;
    --desktop) desktop=1 ;;
    --force) force=1 ;;
    -*) die "unknown option '$arg' (see --help)" ;;
    *) program_arg="$arg" ;;
    esac
done

# The icon, so that this is the one file to hand over: the 128x128 PNG of
# assets/icons (regenerate with: base64 -w 76 assets/icons/icon-128.png).
icon_base64() {
    cat <<'ICON_EOF'
iVBORw0KGgoAAAANSUhEUgAAAIAAAACACAYAAADDPmHLAAAVHElEQVR4nO2df4xcV3XHP+e+NzM7
a+/sru39FRs7P5y4BAeaxCQOREmBQigFtVSkaig0pEJySpGqUugfkKBUJUBVoaIWhViiTdPSkjag
liI1JEFQBRSHQGKROAlJHMdOYnt/2Ov9ObMz7717+sd9b7w2O7szCzvz1jsfaSx75vnNnXu+9+c7
51xh+QjgARGg8Xtef3/XVaLmOuBC4HLgPGBzfI38Et+3lkjq6ihwDNgPHFKxj4yOTj+Oq3NY2AYN
sVyDJF8KQH9/1zWi5maBtyjsFJHqfVWXVa42MfOqElVVgQMKj6rYe0dHp/fNu/QMm9R9/2VcL4AF
sgObuj8kwkcRuUYEVKsGj866vt3yl4fOewF4IkJS16juU+VrIycmvw5UAHPW9UvSiGGqCuvvL9xk
VD4tRnbGRlecKCQuRJuVw+IMbERcX6tWD1jRz4+OTn0jvqbu3qBeAXhAtHFj/ryMyX4JkT8AUNWI
ttFbiQVURDwAVO8LbOUvTp4sHaNOEdQjAA+I+voKNxjkHiMyZNuGTxsWUCPiWdXjFr1lbGzqQeoQ
wWIGrM4w+/sLezwx3xVhyKqG8ftt46cHA3hWNRRhyBPz3f7+wh6c8T0Waei1PkiMHw70dd8tRvao
bbf6VYIbFox4anXvyNjkrYBPjaViLWMaYuMbZ/wwfq9t/PRjAKNWQ2Nkz0Bf991AYr8FLz4b1+1v
LOwxRvZYW+3y20u51YMAno1F0L/xjOHgFy6cjzP+hsI7jW8eUm0bf5WjQCQivg3tu0bHpx7mrInh
fMMagIGBdZvE+k8p9Mc3aHf7qxsLiMComvCNIyOzJ+a9f4ZxBbBi/a+IyEB8Qdv4qx8DWBEZEOt/
hdMbdtUPIe4WBvu6bxQjN85b6rU5N3BLRCM3DvZ138i8+UAiAAVyCneqtrv9cxSjiircCeSIl4QG
t0a0A33dHzRGLlbVdtd/bmJU1RojFw/0dX8QNxT4ydMjo/CxuPW3OYeJe4GPET85NEA0uLFwpRG5
In6q1x77z108VVUjcsXgxsKVQGQAVOQWETdbbG352jQBK4JRkVsAzHY3+XubtdU5QZtzG2OtovC2
7ZCT/v6u3aLmRzjjt3f81gYKWBV7rVE118UOBe3uf+1gRcRTNdcZQXe0ujRtWoOgOwzIzuq/26wV
YlvLTh90c/zv1ApAABO7R8syShnZxjc4BPBM41+WeMHbdG+qxD9MN8tAX3dqy5kYvRJZypElUlex
jaDA+oxH1kjdBhEgsMp0EDXcKowInkDOM2Q9t6hqtMzNxCeFETuCC4iYrkQoytauHJf0drC1K0dv
zifjSV0FVoWsJ3zzxXEOTs7R4ZkljWFEmIssF3V3cOPFG6hEWlevo+pEM1EOOTJd5oVTc7wyXUYQ
urLGxUzU9eubivqkzPhGILRQDELetqWbmy/tY/fgevo6M2SNxMaor8iqIBnD/tFZnhkvkveW3uYQ
oBxZLijk+Mvdr0MD28Cw4/ZZK1YZKwb8eHiGe58b4/uvTtLpe/gGbLpUIH6rSzAfI1COlJwn/P31
F/Ch12/CN0IxsBQDy2xcwfViFdaHhorVhlSeDAGlUsBMYGlkKiACgtCT83n/9g2878Je/u3nJ7jt
0VeYi39bmkSQGgEIEFqlwzP8yw0X8Y5tPZwqBUkITGwEaai/sgq+qW+4WKg8vhF8Iw0JICFUNxwI
cMvOfrYVcvzRgwepRBZP6p+PrDSp2foVEUqh8vm3vo53bOthrBjEE6rlGbDVCOCJYEQYKwa8fWs3
X3jrVkqhnhHw2WpSIQATT/h+c2s3f/hrmxgvBWTraHaqEKkSWiVa6BV/tpzWprgeKdKF7518Vs+Q
lDXCeCngph0beee2bqYrUXWF02pSMwQA3HzpprorJlI3XOR9L25Rv2gJq2AyHpkGloDEd8oYIZ/P
kAuiGkOAoKqUQstc3K0vhYhw8+v7ePjIZAOlWVlaLgARN+veWsjy5oH1FIPFW0eyZu3J+bw0Mcdj
wzMcmSozF9nq59Vr42XgwYk5cp7B1iEDi5LzDC9NznHHj478wjIw+WuHZ9hayLJ7sIvtPR3xkrX2
FMWIUAoi3jywnm2FLMdnA7KeNDSpXQlaLgCDUI4iLunJ05fPMLvErFtwRv3CT46y9+kRTpZCIl24
6l2/oKzPeORMfZWtCjkjHJkq88WfHEWo1Xsonggb8z57LhvgE1cMUYlqf4FbWcCmvM+O3jyHp8p0
eD5Ri6eDLRcAQKSwrSsbd9W121GkSk/O53OPv8Yd+15jUz5DT86vuU5P7tToVnAyBGzKZxZt1apQ
DpXb971KZJXbrt7CRDmsORwoSsYIW7tyLKKVptLySaDLdqH0dPinM18sgFXo9D0OnCxy189G6O/M
4JvTk8CFXslk7ZeaBNa4dzIJ9Az05zPc9dQIT58o0ul7Ndf5qu739uQ8VOvbYVxpWi6ABDfrX2Ts
V6XDFx54eYLJcoRv0rGhkuw1TJYjHjg8QYcvS+RFkuozgjSQnpIsgYgQWmX/2Cyeqd1TtAJV8Azs
H5sltEuv81PQ8KusGgEYgWJgGS4G8U5aehSg8YRweDag2ODWcatZFQJQ3NhZsZraCjYiFMOIilWM
pPLJ34KsCgFAvKSL/QHStjmcrBSspu5p35KsGgG0WRnaAljjtAWwxmkLYI3TFsAax2/1fLrRbNJC
4/9npVlOedLyG9o9wBonNQJIw4ORZiHz/mw1fqvLkTh95uNkJUsKIS1953zqLJP7bUo+Y07vFrb4
d6SiBzAGBtdlV90u2nKwFgY7s5hU1HwKBBCpUsh67OjNx/77i7mDadWRInVakcTxpLYLiSBUrLKj
t4NC1o89mVpLSwXgiQv6eNOmdezo7WAurP2gx4gwG1imgwgvSW2VFhQ8genALurSZgTmIsuO3jy/
3tfJbFCfM+lK0jIBJI90I5Q/fdMAGSM14/YS586Xp8qMlwL8Br18VxoFfBHG5wJeniwvGv1jVfEN
fOyNg1i0+ii5VTRNAEmQhxdH+cwGEeNzEbdftYUbtvUyXYlqVoRV50v3/VcnKUWKafXMaQFcUKny
vVcnnEBriNmLYyBu2NbDZ6/awqm5iNnY9fx0/TTv98lgf3PCw4uhrfrndXjCpRs6+bPLB3n/RRuY
CWxNk1qFnCeMlQLe9V/PMV4OyaQotCpBcOFg3Vmfh97/egY6M5QjrTkcuLB1w3+/NM6X9w/z7HiJ
ucjVgy9CZ6Y5bbMpAhBg18A6tqzPcX4hx5X969g1sJ6ujMdkJaypeNeNKr0dPh/93iHue/4EPbl0
TJ4WwhNhohzy+xdv5B/feRETZZeVvZYIrEIh6zETRPx0ZIYnRmc5Ml3m1ekKPx2ZaYrIV1QAiZNE
xhO+/3uXcvHGTmxksarMBtZ51S5g/CTkK+sJXVmfv/7xa/zNE0fpScnMeTESEXzyyvO4Y/cWZiou
uYUnsuAeR1IH6zLGBcF6hoPjRd7+rWepRCvvXdS0uIDZwDJbDikFFs+cnhOcjRHI+S67xlgx4DOP
vsw9z45RyHqpNz7Ey9qcz5eePM5YMeCzu7cw0JmtZjk5e3KY1MF0JSKykM8YZoPmJWzzm7MVJacn
OYuEWwtQCpXnxmd59PgM//7zEzw7XqI756c6zcrZqEIh63Pvcyd4bGSGD+7YxLVDXVzQ3UGHt/D8
xYiAcctJNyQ2ZyKYisggiJdSRjhVDLjvhZN84/mTjJVCBjv9VHkA14/Sk/N4aqzEsZnjvHpJhT+5
rJ8tXTmCBhNWrCRNE4A7Yrb250lWjs3rsvzttdv45BVD/NOzY3zlZyMEVsl7ZlUMAeB6urnI4ovw
2as388dv6GOwM0sptEsaX2luzEPT9gFynpCNkztFqgsaM1lKTVZCunM+t129hf98z8X05TMUQ5ua
mPrFMCKUQsumDp//eM92bt+9he6cz2QlJNSFjZ/Uh+AipHJe835nUwSgCkemKwwXAyJ1od3dWT9O
WHvmtUlmjcAqJ4oB123u4us3XEQhawhs7f2CNOB6Mcv6rOFf372d67cUOFEMCCNdMNNJkjmsO+vT
k/OxCsPFgCPT5aYNejLY39OU78oYIecZ+vI+Ozd28t4Le3jX1h6s6qIbJhWr9HVmuPeZUT7+f4cp
ZNM7ITQiTFVC/uH68/nIzn7GirUznSQbXEaEh1+Z4DuHJnjmZJHRUkg5ckNFM2iaAFRd8oXQajXp
wg3bevi767axqcNnbhERWIUOX/jd77zAY8MzrM94qROBEWEmiLh6cD3ffu8llBfJL2jV7YaemAv5
xCNH+O7hCSxOEL4RDAvvGaxIuZvzNW5978eOHz051+V959Apbn7oJYqhXfQBj6LkfcMHLt6Qqhn0
fJJJ7Ae2byCfqZ2NJHlwVAqVjzz0Ev9z6BTdHT69OZ+8b/BleVnJlkvTBKBQHfOTmP7Bziw/OjbN
V58aoStbO4unIMyFyq7+dfTkvOqEKS0kE9uerMeugXWUw9p+DVaVrqzhq08P88Nj0wx2ZqsJraye
rqdm0VJ/gNBaClmPbx0cZ7QY1EzmJOKSNfR3ZtiQ8934mDIFhNY9sxjozLheaoHyJZlHRksB33xx
nELWI7StPaahpQJIxr1Xpis8O16iwzM118AuvYqhw699TStx47ohY0zNjSuNr3lufI5XpivOb6DJ
5TyblruECUI5shyaLMfzgMUTLcm8v6eNpSZuiuIb4dDkHOXIpiLKueUCANd6xufCRXMEnQskOYLG
y2FqHGBbLgAR1zLcurf1LaIZBJFWk160mpYLoE1raQtgjdMWwBqnLYA1TlsAa5y2ANY4bQGscdoC
WOOsGgEoLox8qe3iVuDOJXBl81ZRllBYJQJIAkzynqErWzsdeyuxqnRl3MOqtD2sXIxVIQBwFdzh
G7Z1ZV1G7hRVseAymW/typH3lz6dNE2kRgD1tGoBdg92pWYfPUHibn/30Pq6KjRN8mi5ANwjIOdK
vViOoCRBxLvP72ZbIcdsYMks81DIXxWCc/AoBpZtXTl+6/ye+KTRGhlC4rfdb00HLRcA8SPS0WKw
aC8gQMVaBjuz3HnN61CUU+XIjbfifA5rvZYjEmGJe4rrtVwZlM+9ZQtDnVkqS7iuW4WRYuCCPlPQ
FbQ8NCw5SOmlybn4LIDa1efFbtfvu7CXb/72JXz5yeM8fbLIdMUSLlKbOU/wG8gpkASolMPa/8MX
dyr4roEu/vzyIa7fUmCyUvvAKIjPFAgsL03OzTsgq7W0XgCxm9QLE3McnipzUU/OOVUuMhRMV0Ku
O6+La4e6eHmqzPHZCpUFug+rkPeFOx8/xk9GZlhXhzu5G2oidg2s47arNlMKF3ZXzxphaF2G8wsd
Lj/QEsZXhZwvHJos8/ypuUXd35pJ6wWAWz+fLIU8cHiCT+0aohiE+Eu0pJkgQhC2FbJs78ktuCqw
CpmM4e6nRmuGZZ1N0vo35DL8xtYeghpJnxInlrnQZfpZKmwtUiXvezxweIKTpZANHenIddByAYBb
4q3LGP75uTFu2rGR3g6/mhyhFkmFl0NlroYXvlVYr6Zu4yckIpirhDWPj3f+iVI9Ln4xXGCL4bXp
Cvc8O8a6THqWiq2fBOJ6gZxnODJV5vZ9r5H3XSbNelqIyJkJqBZ6LXcSuNg9TY2MH2cTqRNy3jfc
vu81jkyVyXkmBaO/IxU9AJw+FfT+F0+yocPji2/digVmKlHVyCla+i+KC4BxJ4uvz3gYgU/98BXu
f/Fk6nIcpUYA4ETQnfPY+/QoL09V+Kvdm7lsYyeRugSLodVqRG09JIc4Lae6kyimRradBdcjZYzQ
4Rk8EQ6cLHHHY6/xwJGJalRTmkiVAMBVeE/O53uvTPL4yAy/c0Ev77mgh0s35Ont8OnwDfWGz1uF
XMY0tASE0/F7uThVW72xeolQR0sBz50s8b+HJ/j2oVNMliN6U9byE5oWHdwongihKtMVd0xsf2eG
oc4MhayH30D0pCfCM+NFxufCuoRwehXg84aNnQ0ZLbTKVCViuBgwWgwIrNKVceVNo/EhxQKAZDdO
qkuuIHLJohvdQEmibhvdCGp0y1Zwj4MznsTb1C79bWormBQOAfNRTq8EfBEymWTx1RiNGiEZArqz
jVWPxn9qde6QZtM7Ui2A+ZxOntScSp0vvnOZVOwDtGkdacu836a5qGH1eC+1+dUjRlWPSnKaUZu1
gooIqnrUCHI0ebOlRWrTTBRAkKMG4cD8N9usCZythQNGVZ5vcWHatAhVed74Ej6iqhHtJeFawqhq
5Ev4iMkXpver8mLsO5ked9U2K4UVEFVezBem95uDBykbkR/Ej7zaAjj3sRjBiPzg4EHKBkBNdA+K
pT0MrAUMilUT3eP+Ad7w8NQT1uqT4jYEotaWr80KEomIWKtPDg9PPQHuENZ47Ne7VpHXVZtl4mys
d+GG+6pbo9kOmen+nqeNsD1+etoeDs4trAhilYNdoxOXHYQAsImR5SCURfUzuGGgPRk897CIiKh+
5iCUiZ8BJQKIAG94bPJ+tXq/MeLTngucS0TGiK9W7x8em7wf8IjtO7+bV8CoBB+3qiPxZ+2eYPVj
AWNVR1SCj3OWC4A560IZGZkdtWo/LKeHgvYzgtWLAlZExKr98MjI7ChnbfidPdGLAG90dOph1ehW
OT0UtEWw+lAgEiO+anTr6OjUw8zr+hMWmum7+cDo1F4b6V7TFsFqRInHfRvp3uHRqb0sYHyo7Q0k
8X8IB/p67vaM7LHugZHQXh6mHQuoEfEiq3tHxiZuxTn/LtiIF9v4SYwdDfb37hHh7tjlOWQVeROv
MUIRdyC4KrcOj55KWn7NuVw9O38eEA31FW5QMfcYkaF2b5A6qq3eqh4XtbccH5t6kBrd/nzqMWAE
eMfHph4MIm+XVb1PRDwRMfFn7aVi67C4/X0jzvj3BZG3q17jQ2MewdUbDvX33gT6aTGy050KrhoX
pt0rrDxJd25E3GJdrR4A+fzx0VPfiK+py/jQuEt4cnCXBbIDA90fEuSjglyTHPjktFD9cpn3atM4
Ou8F4EmcmMKFyes+Rb82MjL5daDC6U2euldsyzXMGQo7r7/3mkj1ZjH6FlR2xptI7hesgfCqlWRe
VbqeVvSAWnnUE7n32OipffMurbvVn3H/X6Zs8760qtDz+nuvipTrjNELrXK5oOeBbCbJCdmmHuK6
0qOKHDPCfmvlkCc8cmz01OOc2cOebYOG+H+P06jIP4cQ9wAAAABJRU5ErkJggg==
ICON_EOF
}

"cmd_$command"
