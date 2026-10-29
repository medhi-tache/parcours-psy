#!/bin/zsh
set -eu
cd -- "${0:A:h}"
export PATH="$HOME/.cargo/bin:$PATH"
exec node scripts/start.mjs
