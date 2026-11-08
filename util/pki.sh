#!/bin/sh
set -eu

DIR="${VPORT_PKI_DIR:-./pki}"
umask 077
mkdir -p "$DIR"

usage() {
    echo "usage: pki.sh init | server NAME HOST | client IDENTITY" >&2
    exit 1
}

new_key() {
    openssl genpkey -algorithm EC -pkeyopt ec_paramgen_curve:P-256 -out "$1"
}

sign() {
    name="$1"
    extensions="$2"
    new_key "$DIR/$name.key"
    openssl req -new -key "$DIR/$name.key" -subj "/CN=$name" -out "$DIR/$name.csr"
    printf '%s\n' "$extensions" > "$DIR/$name.ext"
    openssl x509 -req -in "$DIR/$name.csr" -CA "$DIR/ca.crt" -CAkey "$DIR/ca.key" \
        -CAcreateserial -days 90 -sha256 -extfile "$DIR/$name.ext" -out "$DIR/$name.crt"
    rm -f "$DIR/$name.csr" "$DIR/$name.ext"
    echo "written $DIR/$name.crt and $DIR/$name.key"
}

check_name() {
    case "$1" in
        ""|*[!A-Za-z0-9._@:-]*)
            echo "invalid name: use letters, digits and . _ @ : - only" >&2
            exit 1
            ;;
    esac
}

command="${1:-}"
case "$command" in
    init)
        new_key "$DIR/ca.key"
        openssl req -x509 -new -key "$DIR/ca.key" -sha256 -days 3650 -subj "/CN=VPort Fleet CA" \
            -addext "basicConstraints=critical,CA:TRUE" \
            -addext "keyUsage=critical,keyCertSign,cRLSign" \
            -out "$DIR/ca.crt"
        echo "written $DIR/ca.crt and $DIR/ca.key"
        echo "keep ca.key offline: whoever has it can create any identity"
        ;;
    server)
        name="${2:-}"
        host="${3:-}"
        check_name "$name"
        [ -n "$host" ] || usage
        case "$host" in
            *[!0-9.]*) san="DNS:$host" ;;
            *) san="IP:$host" ;;
        esac
        sign "$name" "$(printf 'basicConstraints=critical,CA:FALSE\nkeyUsage=critical,digitalSignature\nextendedKeyUsage=serverAuth\nsubjectAltName=%s' "$san")"
        ;;
    client)
        identity="${2:-}"
        check_name "$identity"
        sign "$identity" "$(printf 'basicConstraints=critical,CA:FALSE\nkeyUsage=critical,digitalSignature\nextendedKeyUsage=clientAuth')"
        ;;
    *)
        usage
        ;;
esac