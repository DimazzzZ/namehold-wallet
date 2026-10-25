#!/usr/bin/env bash
# check-appimage.sh — check that an AppImage runs for a user other than the one
# who built it, as firejail and the AppImage catalog's test run it.
#
#   scripts/check-appimage.sh path/to/Namehold.AppImage
#
# 1. File modes: every file its owner can execute is executable by everyone,
#    and everything in usr/bin is executable (the 0.6.0 image shipped a 0644
#    daemon and a 0770 launcher).
# 2. The daemon `namehold-syncd` runs as another user, from that user's own
#    extraction of the image, as a user who runs the AppImage gets it (it
#    loops, so `timeout` ending it, exit 124, is a pass; 126/127 is "cannot
#    execute").
# 3. The app itself starts as another user under a virtual display and is
#    still running after 20 s.
#
# Needs sudo (to add the user), xvfb-run and timeout. Exits non-zero, with the
# reason, on the first failure; prints what it checked on success.
set -euo pipefail
img="${1:?usage: check-appimage.sh <file.AppImage>}"
[ -f "$img" ] || { echo "::error::no AppImage at $img"; exit 1; }

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
chmod 0755 "$work"
cp "$img" "$work/app.AppImage"
chmod 0755 "$work/app.AppImage"
(cd "$work" && ./app.AppImage --appimage-extract >/dev/null)
root="$work/squashfs-root"

bad=$( { find "$root" -type f -perm -u+x ! -perm -o+x; find "$root/usr/bin" -type f ! -perm -o+x; } | sort -u)
if [ -n "$bad" ]; then
  echo "::error::files in the AppImage another user cannot execute:"
  echo "$bad"
  exit 1
fi
[ -x "$root/usr/bin/namehold-syncd" ] || { echo "::error::no executable usr/bin/namehold-syncd in the AppImage"; exit 1; }
echo "file modes OK ($(find "$root" -type f | wc -l) files)"

# For the record: the directory modes on the way to the daemon. They do not
# matter to a user, who extracts or mounts the image as themselves.
stat -c '%A %n' "$root" "$root/usr" "$root/usr/bin" "$root/usr/bin/namehold-syncd"

user=nhappimagecheck
id "$user" >/dev/null 2>&1 || sudo useradd --create-home "$user"
as_user() { sudo -u "$user" -H env HOME="/home/$user" "$@"; }
own="/home/$user/extract"
as_user rm -rf "$own"
as_user mkdir -p "$own"
as_user sh -c "cd '$own' && '$work/app.AppImage' --appimage-extract >/dev/null"

set +e
as_user timeout 10 "$own/squashfs-root/usr/bin/namehold-syncd" >"$work/daemon.log" 2>&1
rc=$?
set -e
case "$rc" in
  0 | 124) echo "namehold-syncd runs as another user (exit $rc)" ;;
  *)
    echo "::error::namehold-syncd failed as another user (exit $rc):"
    cat "$work/daemon.log"
    exit 1
    ;;
esac

set +e
as_user timeout 20 xvfb-run -a "$work/app.AppImage" --appimage-extract-and-run >"$work/app.log" 2>&1
rc=$?
set -e
if [ "$rc" -ne 124 ]; then
  echo "::error::the AppImage exited (code $rc) within 20 s as another user:"
  cat "$work/app.log"
  exit 1
fi
echo "the app starts as another user and is still running after 20 s"
