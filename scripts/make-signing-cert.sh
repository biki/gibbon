#!/bin/bash
# Make the self-signed "Gibbon Release" certificate that signs the releases.
# Run it once:
#   scripts/make-signing-cert.sh [folder]   → folder (default ~/.gibbon-signing)
# Imports the certificate into the login keychain, so scripts/bundle.sh signs
# local builds with it too, and prints the commands that store it as GitHub
# secrets for .github/workflows/release.yml.
# Keep a copy of the folder. Installed apps accept an update only when this
# certificate signed it: with a new certificate, users must install again.
set -euo pipefail
name="Gibbon Release"
dir="${1:-$HOME/.gibbon-signing}"
p12="$dir/gibbon-release.p12"
if [ -e "$p12" ]; then
  echo "$p12 exists. Updates need the same certificate, so keep it." >&2
  exit 1
fi
mkdir -p "$dir"
chmod 700 "$dir"
umask 077

# macOS's own openssl (LibreSSL) writes a .p12 that `security import` reads.
# The .p12 of OpenSSL 3 needs its -legacy flag.
ssl=/usr/bin/openssl
cat >"$dir/cert.cnf" <<CNF
[req]
distinguished_name = dn
x509_extensions = ext
prompt = no
[dn]
CN = $name
[ext]
basicConstraints = critical, CA:false
keyUsage = critical, digitalSignature
extendedKeyUsage = critical, codeSigning
subjectKeyIdentifier = hash
CNF
"$ssl" req -x509 -newkey rsa:3072 -nodes -days 7300 -config "$dir/cert.cnf" \
  -keyout "$dir/key.pem" -out "$dir/cert.pem" 2>/dev/null
"$ssl" rand -base64 24 | tr -d '\n' >"$dir/password"
"$ssl" pkcs12 -export -name "$name" -inkey "$dir/key.pem" -in "$dir/cert.pem" \
  -out "$p12" -passout file:"$dir/password"
rm "$dir/key.pem" "$dir/cert.cnf"

# macOS can ask once to let codesign use the key: choose Always Allow.
security import "$p12" -k login.keychain -P "$(cat "$dir/password")" -T /usr/bin/codesign

cat <<DONE

Made $p12 and its password in $dir/password.
Certificate SHA-1: $("$ssl" x509 -in "$dir/cert.pem" -noout -fingerprint -sha1 | cut -d= -f2)

Store it as GitHub secrets for the release workflow:
  base64 -i "$p12" | gh secret set RELEASE_CERTIFICATE
  gh secret set RELEASE_CERTIFICATE_PASSWORD < "$dir/password"
DONE
