#!/usr/bin/env bash

set -euo pipefail

readonly revision="d62b18db58ccad963ae57b93c12b1f43c0a47f6d"
readonly cache_root="${XDG_CACHE_HOME:-"$HOME/.cache"}/copilot-mcp"
readonly server_dir="$cache_root/c4-structurizr-mcp-server-$revision"

if [[ ! -f "$server_dir/dist/server.js" ]]; then
  mkdir -p "$cache_root"
  git clone --quiet \
    https://github.com/psousa50/c4-structurizr-mcp-server.git \
    "$server_dir"
  git -C "$server_dir" checkout --quiet "$revision"
  npm --prefix "$server_dir" ci --quiet
  npm --prefix "$server_dir" run build --silent
fi

exec node "$server_dir/dist/server.js"
