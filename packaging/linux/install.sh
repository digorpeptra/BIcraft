#!/bin/sh
# Installs bicraft for the current user.
#
#   ./install.sh [DIR]
#
# Copies bicraft from this folder to DIR (default ~/.local/bin). It does not change PATH, shell
# profiles or any MCP client configuration: it prints what to do instead.
set -eu

dest=${1:-"$HOME/.local/bin"}
here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
if [ ! -f "$here/bicraft" ]; then
    echo "install.sh: bicraft not found next to install.sh ($here)" >&2
    exit 1
fi
mkdir -p "$dest"
dest=$(CDPATH= cd -- "$dest" && pwd)
# Copy then rename, so a running bicraft (an MCP server) is replaced safely.
cp "$here/bicraft" "$dest/.bicraft.tmp.$$"
chmod 755 "$dest/.bicraft.tmp.$$"
mv -f "$dest/.bicraft.tmp.$$" "$dest/bicraft"
echo "Installed $("$dest/bicraft" --version)"
echo "  -> $dest/bicraft"

case ":${PATH:-}:" in
    *":$dest:"*) ;;
    *)
        echo
        echo "$dest is not on PATH. Add this line to your shell profile (~/.profile, ~/.bashrc):"
        echo "  export PATH=\"$dest:\$PATH\""
        ;;
esac
echo
echo 'MCP clients: run `bicraft setup --dry-run` to see the registration plan, then'
echo '`bicraft setup --client claude_code --yes` (or another client); see docs/mcp.md.'
