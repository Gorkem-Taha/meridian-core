#!/usr/bin/env bash
# Publishes the example module to real OCI registries and uses it from a
# project, the way the documentation says to.
#
# It expects an open registry on localhost:5000 and one that wants the login
# reader / s3cret on localhost:5001, and `oras` on the PATH. The modules
# workflow starts both; see .github/workflows/modules.yml.
#
#   tools/real_registry_test.sh /path/to/meridian
set -euo pipefail

meridian=${1:?usage: real_registry_test.sh /path/to/meridian}
root=$(cd "$(dirname "$0")/.." && pwd)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

layer=application/vnd.meridian.module.layer.v1.tar+gzip
artifact=application/vnd.meridian.module.v1

say() { printf '\n== %s\n' "$*"; }

fail() {
  echo "FAILED: $*" >&2
  exit 1
}

# Publishes the example module as the given version to a repository.
publish() {
  local repository=$1 version=$2
  local src="$work/src-$version"
  cp -r "$root/examples/modules/hardening" "$src"
  sed -i "s/^  version: .*/  version: $version/" "$src/meridian.yaml"
  (cd "$src" && "$meridian" module pack --out "$work/module-$version.tar.gz")
  (cd "$work" && oras push --plain-http "$repository:$version" \
    --artifact-type "$artifact" "module-$version.tar.gz:$layer")
}

# Writes a project that requires the module from a repository.
project() {
  local dir=$1 repository=$2 requirement=$3
  mkdir -p "$dir"
  cp "$root/examples/modules/site/web.yaml" "$root/examples/modules/site/data.yaml" "$dir/"
  cat > "$dir/meridian.yaml" <<YAML
apiVersion: meridian/v1
kind: Module
metadata:
  name: registry-test
spec:
  modules:
    - name: acme/hardening
      version: "$requirement"
      oci: $repository
      insecure: true
YAML
}

say "an open registry"
open=localhost:5000/acme/meridian-hardening
publish "$open" 1.2.0
publish "$open" 1.3.0
publish "$open" 2.0.0
# A tag that is not a version, and an image that is not a module, are both
# in the repository and must both be ignored.
(cd "$work" && echo notes > notes.txt && oras push --plain-http "$open:latest" notes.txt:text/plain)

project "$work/site" "$open" "^1.2"
cd "$work/site"
"$meridian" module install | tee "$work/install.txt"
grep -q '^+ acme/hardening 1.3.0 ' "$work/install.txt" || fail "the newest 1.x was not chosen"
grep -q 'resolved: sha256:' meridian.lock || fail "the manifest digest was not pinned"
"$meridian" module verify
"$meridian" module install --locked

# What was fetched is what was packed.
packed=$(cd "$work/src-1.3.0" && "$meridian" module pack --out "$work/again.tar.gz" | sed -n 's/^  integrity:  //p')
grep -q "integrity: $packed" meridian.lock || fail "the pinned hash is not the packed module's hash"

say "a fresh machine fetches the pinned digest"
rm -rf .meridian
"$meridian" module install --locked
"$meridian" module verify

say "a run reads the module"
"$meridian" compile web.yaml @acme/hardening:documents/ssh.yaml \
  --data @acme/hardening:data/defaults.yaml --data data.yaml --out "$work/out"
grep -q '10-no-root.conf' "$work/out/ssh-baseline/playbook.yml" || fail "the module's document was not compiled"

say "a tampered module stops the run"
echo '# edited' >> .meridian/modules/acme/hardening/1.3.0/documents/ssh.yaml
if "$meridian" validate @acme/hardening:documents/ssh.yaml 2> "$work/tamper.txt"; then
  fail "a changed module was accepted"
fi
grep -q 'does not match meridian.lock' "$work/tamper.txt" || fail "the refusal does not say why"
"$meridian" module install --locked
"$meridian" validate @acme/hardening:documents/ssh.yaml

say "an image that is not a module is refused by name"
project "$work/not-a-module" "$open" "=9.9.9"
(cd "$work" && oras push --plain-http "$open:9.9.9" notes.txt:text/plain)
cd "$work/not-a-module"
if "$meridian" module install 2> "$work/image.txt"; then
  fail "something that is not a module was installed"
fi
grep -q 'is not a Meridian module' "$work/image.txt" || { cat "$work/image.txt"; fail "the refusal does not say why"; }

say "a registry that wants a login"
private=localhost:5001/acme/meridian-hardening
export DOCKER_CONFIG="$work/docker"
mkdir -p "$DOCKER_CONFIG"
project "$work/private" "$private" "^1.2"
cd "$work/private"
if "$meridian" module install 2> "$work/denied.txt"; then
  fail "a private registry was read without a login"
fi
grep -q 'no login for it was found' "$work/denied.txt" || { cat "$work/denied.txt"; fail "the refusal does not say why"; }

# The login is written the way `docker login` writes it.
printf '{"auths":{"localhost:5001":{"auth":"%s"}}}\n' "$(printf 'reader:s3cret' | base64)" > "$DOCKER_CONFIG/config.json"
(cd "$work" && oras push --plain-http -u reader -p s3cret "$private:1.2.0" \
  --artifact-type "$artifact" "module-1.2.0.tar.gz:$layer")
"$meridian" module install | tee "$work/private.txt"
grep -q '^+ acme/hardening 1.2.0 ' "$work/private.txt" || fail "the private module was not installed"
"$meridian" module verify

say "all passed"
