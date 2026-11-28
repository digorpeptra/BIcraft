#!/bin/sh
# Installs BIcraft for the current user.
#
#   ./install.sh [PREFIX]
#
# Copies this folder's bin/ and share/ under PREFIX (default ~/.local): the desktop app
# PREFIX/bin/bicraft, the CLI and MCP server PREFIX/bin/bicraft-cli, the menu entry, the icons,
# the file types (.pbix, .pbit, .pbip), the AppStream metadata, the documentation and the Power BI
# visuals. It refreshes the menu, MIME and icon caches of PREFIX/share when their tools are
# installed. It does not change PATH, shell profiles, default applications or any MCP client
# configuration: it prints what to do instead.
set -eu

prefix=${1:-"$HOME/.local"}
here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
for f in bin/bicraft bin/bicraft-cli share/applications/io.github.digorpeptra.bicraft.desktop; do
    if [ ! -f "$here/$f" ]; then
        echo "install.sh: $f not found next to install.sh ($here)" >&2
        exit 1
    fi
done
mkdir -p "$prefix/bin"
prefix=$(CDPATH= cd -- "$prefix" && pwd)

# Copy then rename, so a running program (an MCP server, the app) is replaced safely.
for b in bicraft bicraft-cli; do
    cp "$here/bin/$b" "$prefix/bin/.$b.tmp.$$"
    chmod 755 "$prefix/bin/.$b.tmp.$$"
    mv -f "$prefix/bin/.$b.tmp.$$" "$prefix/bin/$b"
done

# share/: every file at the same place under PREFIX/share; the menu entry runs the app by its
# absolute path, so it works whether or not PREFIX/bin is on PATH.
(cd "$here/share" && find . -type f) | while IFS= read -r f; do
    f=${f#./}
    mkdir -p "$prefix/share/$(dirname -- "$f")"
    cp "$here/share/$f" "$prefix/share/$f"
    chmod 644 "$prefix/share/$f"
done
desktop="$prefix/share/applications/io.github.digorpeptra.bicraft.desktop"
sed -e "s|^Exec=bicraft |Exec=\"$prefix/bin/bicraft\" |" -e "s|^TryExec=bicraft\$|TryExec=$prefix/bin/bicraft|" \
    "$here/share/applications/io.github.digorpeptra.bicraft.desktop" > "$desktop.tmp.$$"
mv -f "$desktop.tmp.$$" "$desktop"

if command -v update-mime-database > /dev/null 2>&1; then
    update-mime-database "$prefix/share/mime" > /dev/null 2>&1 || true
fi
if command -v update-desktop-database > /dev/null 2>&1; then
    update-desktop-database "$prefix/share/applications" > /dev/null 2>&1 || true
fi
if command -v gtk-update-icon-cache > /dev/null 2>&1 && [ -f "$prefix/share/icons/hicolor/index.theme" ]; then
    gtk-update-icon-cache -q -t "$prefix/share/icons/hicolor" > /dev/null 2>&1 || true
fi

echo "Installed $("$prefix/bin/bicraft-cli" --version)"
echo "  -> $prefix/bin/bicraft      (the desktop app; menu entry $desktop)"
echo "  -> $prefix/bin/bicraft-cli  (the CLI and MCP server)"
echo "  -> $prefix/share/doc/bicraft, $prefix/share/bicraft/visuals"

case ":${PATH:-}:" in
    *":$prefix/bin:"*) ;;
    *)
        echo
        echo "$prefix/bin is not on PATH. Add this line to your shell profile (~/.profile, ~/.bashrc):"
        echo "  export PATH=\"$prefix/bin:\$PATH\""
        ;;
esac
echo
echo 'MCP clients: run `bicraft-cli setup --dry-run` to see the registration plan, then'
echo '`bicraft-cli setup --client claude_code --yes` (or another client); see share/doc/bicraft/mcp.md.'
