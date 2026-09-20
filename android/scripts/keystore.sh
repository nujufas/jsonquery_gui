#!/usr/bin/env bash
# Create the UPLOAD keystore Google Play checks your uploads against.
#
#   android/scripts/keystore.sh
#
# Writes android/signing/upload.jks and android/signing/keystore.properties
# (both git-ignored; the build reads them). With Play App Signing -- which
# Play requires for new apps -- Google keeps the real app-signing key and this
# one only proves an upload is yours, so a lost upload key can be reset through
# Play support. Still: BACK THE FILE AND ITS PASSWORDS UP somewhere safe now.
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"

signing="$ANDROID_DIR/signing"
[ ! -e "$signing/upload.jks" ] || die "$signing/upload.jks already exists; not overwriting it. Move it away first if you really mean to replace it."
mkdir -p "$signing" && chmod 700 "$signing"

in_container --env KEY_DNAME -- bash -c '
set -euo pipefail
cd /work/android/signing
password="$(python3 -c "import secrets,string; print(\"\".join(secrets.choice(string.ascii_letters+string.digits) for _ in range(28)))")"
keytool -genkeypair -keystore upload.jks -alias upload \
    -keyalg RSA -keysize 4096 -validity 9125 \
    -dname "${KEY_DNAME:-CN=jsonquery upload key}" \
    -storepass "$password" -keypass "$password" >/dev/null
umask 077
cat > keystore.properties <<PROPS
storeFile=signing/upload.jks
storePassword=$password
keyAlias=upload
keyPassword=$password
PROPS
echo
keytool -list -v -keystore upload.jks -storepass "$password" | grep -E "Alias name|SHA1:|SHA256:|Valid from"
'
chmod 600 "$signing/upload.jks" "$signing/keystore.properties"

cat >&2 <<MSG

[android] Created android/signing/upload.jks and android/signing/keystore.properties.
[android] Back both up now (a password manager or an encrypted drive). Never commit them.
[android] For CI, store the keystore as a secret: base64 -w0 android/signing/upload.jks
[android] Next: android/scripts/build.sh aab && android/scripts/preflight.sh
MSG
