#!/bin/sh
set -eu

here=$(cd "$(dirname "$0")" && pwd)
uninstall=false
if [ "${1:-}" = "--uninstall" ]; then
    uninstall=true
    shift
fi
prefix=${1:-"$HOME/.local"}

files="bin/zhell bin/zhelld share/applications/zhell.desktop share/metainfo/eu.zsync.zhell.metainfo.xml"
for size in 16 24 32 48 64 128 256 512; do
    files="$files share/icons/hicolor/${size}x${size}/apps/zhell.png"
done

if $uninstall; then
    for f in $files; do rm -f "$prefix/$f"; done
    echo "Zhell removed from $prefix. Your settings and history stay in ~/.config/zhell and ~/.local/state/zhell."
    exit 0
fi

install -Dm755 "$here/zhell" "$prefix/bin/zhell"
install -Dm755 "$here/zhelld" "$prefix/bin/zhelld"
install -Dm644 "$here/zhell.desktop" "$prefix/share/applications/zhell.desktop"
install -Dm644 "$here/eu.zsync.zhell.metainfo.xml" "$prefix/share/metainfo/eu.zsync.zhell.metainfo.xml"
for size in 16 24 32 48 64 128 256 512; do
    install -Dm644 "$here/icons/zhell-$size.png" "$prefix/share/icons/hicolor/${size}x${size}/apps/zhell.png"
done
command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database -q "$prefix/share/applications" 2>/dev/null || true
command -v gtk-update-icon-cache >/dev/null 2>&1 && gtk-update-icon-cache -q -t "$prefix/share/icons/hicolor" 2>/dev/null || true

echo "Zhell installed to $prefix."
case ":$PATH:" in
    *":$prefix/bin:"*) echo "Start it with: zhell" ;;
    *) echo "Add $prefix/bin to your PATH, or start it with: $prefix/bin/zhell" ;;
esac
