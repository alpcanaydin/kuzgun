#!/bin/bash
# cargo runner (.cargo/config.toml): sign, then exec the binary.
"$(dirname "$0")/sign-dev.sh" "$1" || true
exec "$@"
