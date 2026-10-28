#!/usr/bin/env bash
# Integration test on a private session bus: fmrelay runs a recorder instead of a file
# manager, and the test checks what the recorder was started with.
#
# Needs dbus-run-session (from dbus) and busctl (from systemd). Never touches the real
# session bus, and the private bus has no service directories, so a call can't start
# Nautilus or anything else through activation.
set -euo pipefail

NAME=org.freedesktop.FileManager1
OBJECT=/org/freedesktop/FileManager1

if [[ ${1-} != --inside ]]; then
    root=$(cd "$(dirname "$0")/.." && pwd)
    cargo build --quiet --manifest-path "$root/Cargo.toml"
    tmp=$(mktemp -d)
    trap 'rm -rf "$tmp"' EXIT
    cat >"$tmp/bus.conf" <<'EOF'
<!DOCTYPE busconfig PUBLIC "-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN"
 "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
<busconfig>
  <type>session</type>
  <listen>unix:tmpdir=/tmp</listen>
  <auth>EXTERNAL</auth>
  <policy context="default">
    <allow send_destination="*"/>
    <allow receive_sender="*"/>
    <allow own="*"/>
  </policy>
</busconfig>
EOF
    dbus-run-session --config-file="$tmp/bus.conf" -- \
        "$0" --inside "$tmp" "${CARGO_TARGET_DIR:-$root/target}/debug/fmrelay"
    exit
fi

tmp=$2
fmrelay=$3
out=$tmp/out
mkdir "$out"

# Writes one line per start: its arguments in brackets and the activation variables,
# then stays alive for a while so we can tell whether the D-Bus call waited for it.
recorder=$tmp/recorder
cat >"$recorder" <<'EOF'
#!/bin/sh
line=$1
shift
for arg; do line="$line [$arg]"; done
line="$line XDG_ACTIVATION_TOKEN=${XDG_ACTIVATION_TOKEN-unset}"
line="$line DESKTOP_STARTUP_ID=${DESKTOP_STARTUP_ID-unset}"
f=$(mktemp "$OUT/.call.XXXXXX")
printf '%s\n' "$line" >"$f"
mv "$f" "$OUT/${f##*/.}"
sleep 2
EOF
chmod +x "$recorder"

failed=0
pass() { echo "ok   $1"; }
fail() {
    echo "FAIL $1"
    failed=1
}

# Stale tokens in fmrelay's own environment must not reach the children.
XDG_ACTIVATION_TOKEN=stale DESKTOP_STARTUP_ID=stale OUT=$out "$fmrelay" \
    --folders "$recorder folders {}" \
    --items "$recorder items --select={}" \
    --properties "$recorder properties {parent}" \
    2>"$tmp/log" &
pid=$!
trap 'kill $pid 2>/dev/null || true' EXIT

for _ in $(seq 50); do
    busctl --user --no-pager status "$NAME" >/dev/null 2>&1 && break
    sleep 0.1
done
busctl --user --no-pager status "$NAME" >/dev/null 2>&1 || {
    echo "FAIL fmrelay never took $NAME"
    cat "$tmp/log"
    exit 1
}

slowest=0
# call METHOD STARTUP_ID URI...
call() {
    local method=$1 startup_id=$2 start elapsed
    shift 2
    start=$(date +%s%N)
    busctl --user --no-pager --auto-start=no call "$NAME" "$OBJECT" "$NAME" "$method" ass "$#" "$@" "$startup_id"
    elapsed=$((($(date +%s%N) - start) / 1000000))
    ((elapsed > slowest)) && slowest=$elapsed
    return 0
}

call ShowItems token123 \
    'file:///tmp/fmrelay%20test/%E4%B8%AD%E6%96%87%20%E6%AA%94.txt' \
    'file://localhost/tmp/-rf'
call ShowFolders '' 'file:///a' 'sftp://host/b' 'file:///c%2Fd'
call ShowItemProperties token456 'file:///a/x' 'file:///a/y'
call ShowFolders token789 'sftp://host/only'

expected=$(
    sort <<'EOF'
items [--select=/tmp/fmrelay test/中文 檔.txt] XDG_ACTIVATION_TOKEN=token123 DESKTOP_STARTUP_ID=token123
items [--select=/tmp/-rf] XDG_ACTIVATION_TOKEN=token123 DESKTOP_STARTUP_ID=token123
folders [/a] [/c/d] XDG_ACTIVATION_TOKEN=unset DESKTOP_STARTUP_ID=unset
properties [/a] XDG_ACTIVATION_TOKEN=token456 DESKTOP_STARTUP_ID=token456
EOF
)
want=$(wc -l <<<"$expected")

# Wait for every expected start, then a little longer to catch extra ones.
for _ in $(seq 50); do
    (($(find "$out" -name 'call.*' | wc -l) >= want)) && break
    sleep 0.1
done
sleep 0.5
got=$(find "$out" -name 'call.*' -exec cat {} + | sort)

if [[ $got == "$expected" ]]; then
    pass "commands and activation tokens"
else
    fail "commands and activation tokens"
    diff <(echo "$expected") <(echo "$got") | sed 's/^/     /' || true
fi

# The recorder sleeps 2 s, so a call that waited for it would take at least that long.
if ((slowest < 1000)); then
    pass "calls return right away (slowest ${slowest} ms)"
else
    fail "calls return right away (slowest ${slowest} ms)"
fi

# Once the recorders exit, fmrelay must have reaped them: no children left, not even zombies.
for _ in $(seq 50); do
    [[ -z $(ps --ppid "$pid" -o pid=) ]] && break
    sleep 0.1
done
children=$(ps --ppid "$pid" -o pid=,stat=,args= || true)
if [[ -z $children ]]; then
    pass "finished children are reaped"
else
    fail "finished children are reaped"
    sed 's/^/     /' <<<"$children"
fi

if kill -0 "$pid" 2>/dev/null; then
    pass "fmrelay still running"
else
    fail "fmrelay still running"
fi

if ((failed)); then
    echo "fmrelay log:"
    sed 's/^/     /' "$tmp/log"
fi
exit "$failed"
