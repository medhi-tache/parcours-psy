#!/bin/sh
set -eu
cd -- "$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
PATH="$HOME/.cargo/bin:$PATH"
export PATH
exec node scripts/start.mjs
