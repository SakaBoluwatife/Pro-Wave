#!/bin/sh
# Installs Ovrtone for the current user (no root needed). Undo with: ./install.sh --uninstall
set -e
here=$(cd "$(dirname "$0")" && pwd)
bin="$HOME/.local/bin"; apps="$HOME/.local/share/applications"
icons="$HOME/.local/share/icons/hicolor/512x512/apps"; mime="$HOME/.local/share/mime"
if [ "$1" = "--uninstall" ]; then
  rm -f "$bin/ovrtone" "$apps/ovrtone.desktop" "$icons/ovrtone.png" "$mime/packages/ovrtone.xml"
  echo "Ovrtone removed."
else
  mkdir -p "$bin" "$apps" "$icons" "$mime/packages"
  install -m 755 "$here/ovrtone" "$bin/ovrtone"
  install -m 644 "$here/ovrtone.desktop" "$apps/"
  install -m 644 "$here/ovrtone.png" "$icons/"
  install -m 644 "$here/ovrtone-mime.xml" "$mime/packages/ovrtone.xml"
  command -v update-mime-database >/dev/null && update-mime-database "$mime" || true
  command -v update-desktop-database >/dev/null && update-desktop-database "$apps" || true
  echo "Installed to $bin/ovrtone. Start it from your app menu or run: ovrtone"
  case ":$PATH:" in *":$bin:"*) ;; *) echo "Note: add $bin to your PATH to run it from a terminal." ;; esac
fi
