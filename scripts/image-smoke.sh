#!/usr/bin/env bash
# Black-box smoke check for a built `home` image.
#
#   scripts/image-smoke.sh <image_tag> [--platform linux/amd64|linux/arm64]
#
# Everything runs in throwaway containers, one throwaway docker volume and a
# temp directory: no real home.sqlite, no real credentials, no device command.
# Each assertion prints PASS or FAIL; there is deliberately no `set -e`, so a
# red image reports every failing check. Any failure makes the script exit
# non-zero.
set -uo pipefail

usage() {
  echo "usage: $0 <image_tag> [--platform linux/amd64|linux/arm64]" >&2
}

IMAGE=${1:-}
if [ -z "$IMAGE" ] || [ "$IMAGE" = "-h" ] || [ "$IMAGE" = "--help" ]; then
  usage
  [ -n "$IMAGE" ] && exit 0
  exit 2
fi
shift

PLATFORM=
while [ $# -gt 0 ]; do
  case "$1" in
    --platform)
      [ $# -ge 2 ] || { usage; exit 2; }
      PLATFORM=$2
      shift 2
      ;;
    *)
      echo "unknown argument: $1" >&2
      usage
      exit 2
      ;;
  esac
done
case "$PLATFORM" in
  ""|linux/amd64|linux/arm64) ;;
  *) echo "--platform must be linux/amd64 or linux/arm64" >&2; exit 2 ;;
esac

# Deliberately not `set -e`: every check runs and the tally decides the exit.
PASS_COUNT=0
FAIL_COUNT=0
ok()  { PASS_COUNT=$((PASS_COUNT + 1)); printf 'PASS  %s\n' "$*"; }
bad() { FAIL_COUNT=$((FAIL_COUNT + 1)); printf 'FAIL  %s\n' "$*"; }

# One line of command output, for the failure message.
trim() { printf '%s' "$1" | tr '\n\t' '  ' | cut -c1-300; }

TMPDIR_SMOKE=$(mktemp -d)
VOLUME="home-smoke-$$-$RANDOM"
CID=
cleanup() {
  [ -n "$CID" ] && docker rm -f "$CID" >/dev/null 2>&1
  docker volume rm -f "$VOLUME" >/dev/null 2>&1
  rm -rf "$TMPDIR_SMOKE"
}
trap cleanup EXIT

# `docker run` wrapper: --rm, plus --platform when the caller asked for one.
# Branch instead of an array so an empty array under `set -u` is not a trap.
dk() {
  if [ -n "$PLATFORM" ]; then
    docker run --rm --platform "$PLATFORM" "$@"
  else
    docker run --rm "$@"
  fi
}

if ! docker image inspect "$IMAGE" >/dev/null 2>&1; then
  echo "image-smoke: image $IMAGE is not loaded (docker image inspect failed)" >&2
  exit 1
fi

echo "image-smoke: image=$IMAGE platform=${PLATFORM:-host}"

# ---- 1. /usr/local/bin/home is statically linked --------------------------
# Static means: the ELF program headers carry no INTERP, the dynamic section
# carries no NEEDED libraries, and `ldd` maps no shared object. glibc's ldd
# says "not a dynamic program"; musl's ldd says "Not a valid dynamic program"
# for static-PIE or just prints the loader address. A dynamic binary always
# prints a "libc.musl-*.so => ..." mapping, so a `=>` line is the red flag.
BIN="$TMPDIR_SMOKE/home"
if ! dk --entrypoint cat "$IMAGE" /usr/local/bin/home >"$BIN" 2>"$TMPDIR_SMOKE/extract.err"; then
  bad "1. static binary: could not extract /usr/local/bin/home: $(trim "$(cat "$TMPDIR_SMOKE/extract.err")")"
elif ! command -v readelf >/dev/null 2>&1; then
  bad "1. static binary: host readelf is missing, cannot inspect the ELF (not skipped silently)"
elif [ "$(head -c 4 "$BIN" | od -An -tx1 | tr -d ' \n')" != "7f454c46" ]; then
  bad "1. static binary: /usr/local/bin/home is not an ELF file"
else
  INTERP=$(readelf -l "$BIN" 2>/dev/null | grep -c INTERP)
  NEEDED=$(readelf -d "$BIN" 2>/dev/null | grep -c NEEDED)
  LDD_OUT=$(dk --entrypoint sh "$IMAGE" -c 'ldd /usr/local/bin/home' 2>&1)
  if [ "$INTERP" -eq 0 ] && [ "$NEEDED" -eq 0 ] \
     && ! printf '%s' "$LDD_OUT" | grep -q '=>'; then
    ok "1. /usr/local/bin/home is static (no INTERP, 0 NEEDED, ldd maps no library)"
  else
    bad "1. /usr/local/bin/home is not static (INTERP=$INTERP, NEEDED=$NEEDED: $(trim "$LDD_OUT"))"
  fi
fi

# ---- 2. base image is alpine ---------------------------------------------
OS_RELEASE=$(dk --entrypoint cat "$IMAGE" /etc/os-release 2>&1)
if printf '%s\n' "$OS_RELEASE" | grep -qx 'ID=alpine'; then
  ok "2. base image is alpine (/etc/os-release ID=alpine)"
else
  bad "2. base image is not alpine ($(trim "$(printf '%s\n' "$OS_RELEASE" | grep -E '^(ID|PRETTY_NAME)=' | tr '\n' ' ')"))"
fi

# ---- 3. container runs as UID 10001 --------------------------------------
UID_OUT=$(dk --entrypoint id "$IMAGE" -u 2>&1)
if [ "$UID_OUT" = "10001" ]; then
  ok "3. container runs as UID 10001"
else
  bad "3. expected UID 10001, got '$(trim "$UID_OUT")'"
fi

# ---- 4. README `--entrypoint sh` password-file flow ----------------------
docker volume create "$VOLUME" >/dev/null 2>&1
if printf '%s\n' 'smoke-test-pass-123' \
     | dk -i -v "$VOLUME:/data" --entrypoint sh "$IMAGE" -c 'umask 077; cat > /data/home-password' 2>"$TMPDIR_SMOKE/pw.err"; then
  PW_META=$(dk -v "$VOLUME:/data" --entrypoint sh "$IMAGE" -c 'stat -c "%a %u" /data/home-password' 2>&1)
  if [ "$PW_META" = "600 10001" ]; then
    ok "4. password file via --entrypoint sh: /data/home-password exists, mode 600, uid 10001"
  else
    bad "4. /data/home-password expected '600 10001' (mode uid), got '$(trim "$PW_META")'"
  fi
else
  bad "4. writing the password file failed: $(trim "$(cat "$TMPDIR_SMOKE/pw.err")")"
fi

# ---- 5. serve: login and API behind it -----------------------------------
PORT=$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1]); s.close()' 2>/dev/null)
if [ -z "$PORT" ]; then
  bad "5. serve: no free loopback port could be picked (python3 missing?)"
else
  # No --rm here: a serve container that dies must still be readable via logs.
  if [ -n "$PLATFORM" ]; then
    CID=$(docker run -d --platform "$PLATFORM" --name "home-smoke-$$-$RANDOM" \
            -p "127.0.0.1:$PORT:8080" -v "$VOLUME:/data" \
            -e HOME_PASSWORD_FILE=/data/home-password "$IMAGE" 2>"$TMPDIR_SMOKE/serve.err")
  else
    CID=$(docker run -d --name "home-smoke-$$-$RANDOM" \
            -p "127.0.0.1:$PORT:8080" -v "$VOLUME:/data" \
            -e HOME_PASSWORD_FILE=/data/home-password "$IMAGE" 2>"$TMPDIR_SMOKE/serve.err")
  fi
  LOGIN_CODE=
  for _ in $(seq 1 60); do
    LOGIN_CODE=$(curl -s -o /dev/null -m 2 --noproxy '*' -w '%{http_code}' "http://127.0.0.1:$PORT/login" || :)
    [ "$LOGIN_CODE" = "200" ] && break
    [ "$(docker inspect -f '{{.State.Running}}' "$CID" 2>/dev/null)" = "true" ] || break
    sleep 0.5
  done
  if [ "$LOGIN_CODE" != "200" ]; then
    bad "5. serve: panel did not answer GET /login 200 (got '${LOGIN_CODE:-no answer}'; container: $(trim "$(docker logs --tail 5 "$CID" 2>&1)") $(trim "$(cat "$TMPDIR_SMOKE/serve.err")"))"
  else
    ok "5a. GET /login returns 200"
    ANON_CODE=$(curl -s -o /dev/null -m 5 --noproxy '*' -w '%{http_code}' "http://127.0.0.1:$PORT/api/devices")
    if [ "$ANON_CODE" = "401" ]; then
      ok "5b. anonymous GET /api/devices returns 401"
    else
      bad "5b. anonymous GET /api/devices expected 401, got '$ANON_CODE'"
    fi
    JAR="$TMPDIR_SMOKE/cookies"
    POST_CODE=$(curl -s -o /dev/null -m 5 --noproxy '*' -c "$JAR" -w '%{http_code}' \
                  -d 'password=smoke-test-pass-123' "http://127.0.0.1:$PORT/login")
    COOKIE=$(awk -F'\t' '$6 == "home_session" { print $7 }' "$JAR" 2>/dev/null)
    if [ "$POST_CODE" = "200" ] && [ -n "$COOKIE" ]; then
      ok "5c. POST /login returns 200 and sets home_session"
    else
      bad "5c. POST /login expected 200 + home_session cookie, got '$POST_CODE' cookie='${COOKIE:-none}'"
    fi
    if [ -n "$COOKIE" ]; then
      AUTH_CODE=$(curl -s -o /dev/null -m 5 --noproxy '*' -b "$JAR" -w '%{http_code}' \
                    "http://127.0.0.1:$PORT/api/devices")
      if [ "$AUTH_CODE" = "200" ]; then
        ok "5d. authenticated GET /api/devices returns 200"
      else
        bad "5d. authenticated GET /api/devices expected 200, got '$AUTH_CODE'"
      fi
    else
      bad "5d. authenticated GET /api/devices not attempted: no home_session cookie"
    fi
  fi
fi

# ---- 6. outbound HTTPS/TLS to the Google auth endpoint -------------------
# A deliberately invalid oauth token: Google answers BadAuthentication over a
# working TLS connection, which the CLI maps to exit 5 / reauth_required. A
# transport or certificate failure (exit 7, "error sending request") is marked
# as a network failure, never as a pass.
TLS_OUT=$(printf 'oauth2_4/smoke-invalid-token\n' \
            | dk -i "$IMAGE" --db /tmp/tls-smoke.sqlite gh login --email smoke@example.invalid 2>&1)
TLS_RC=$?
if [ "$TLS_RC" -eq 5 ] && printf '%s' "$TLS_OUT" | grep -q 'BadAuthentication'; then
  ok "6. TLS to https://android.clients.google.com/auth works (invalid token -> exit 5 BadAuthentication)"
elif printf '%s' "$TLS_OUT" | grep -qiE 'error sending request|certificate|handshake|dns|connect|timed out'; then
  bad "6. TLS/network failure reaching Google (exit $TLS_RC), not a credential answer: $(trim "$TLS_OUT")"
else
  bad "6. expected exit 5 + BadAuthentication from Google, got exit $TLS_RC: $(trim "$TLS_OUT")"
fi

# ---- 7. image size -------------------------------------------------------
# Threshold: docker save tar <= 12582912 bytes (12 MiB). 0.1.0 is ~35 MB and
# fails; the alpine/musl target is ~7 MB.
SIZE_LIMIT=12582912
if [ -n "$PLATFORM" ]; then
  SAVE_CMD=(docker save --platform "$PLATFORM" "$IMAGE")
else
  SAVE_CMD=(docker save "$IMAGE")
fi
if "${SAVE_CMD[@]}" >"$TMPDIR_SMOKE/image.tar" 2>"$TMPDIR_SMOKE/save.err"; then
  TAR_BYTES=$(stat -c %s "$TMPDIR_SMOKE/image.tar")
  GZ_BYTES=$(gzip -c "$TMPDIR_SMOKE/image.tar" | wc -c | tr -d ' ')
  if [ "$TAR_BYTES" -le "$SIZE_LIMIT" ]; then
    ok "7. image size: docker save tar $TAR_BYTES bytes (gzip $GZ_BYTES) <= $SIZE_LIMIT (12 MiB)"
  else
    bad "7. image size: docker save tar $TAR_BYTES bytes (gzip $GZ_BYTES) > $SIZE_LIMIT (12 MiB)"
  fi
else
  bad "7. docker save failed: $(trim "$(cat "$TMPDIR_SMOKE/save.err")")"
fi

echo
echo "image-smoke: $PASS_COUNT passed, $FAIL_COUNT failed"
if [ "$FAIL_COUNT" -eq 0 ]; then
  echo "image-smoke: RESULT PASS"
  exit 0
fi
echo "image-smoke: RESULT FAIL"
exit 1
