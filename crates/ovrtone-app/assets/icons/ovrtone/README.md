# Ovrtone icons

Every icon Ovrtone draws is one SVG here, made by `../make-icons.pl` (`perl make-icons.pl` in the folder
above). `docs/icon-sheet.html` shows them all at 40, 24 and 16 px.

## Replacing an icon (no code change)

Put `<name>.svg` in `%APPDATA%\ovrtone\icons\` and restart Ovrtone: it is drawn in place of the icon of that
name. A file that cannot be read keeps the built-in icon.

## Drawing spec

- `viewBox="0 0 24 24"`, art inside about 2 to 22.
- Lines: `<path d="...">`, drawn 1.75 wide with round ends and joins. Use paths only (M L H V C S Q T A Z).
- Soft tone: `<path data-tone="soft" d="..."/>`, a closed **convex** shape, filled at about a third of the
  icon's colour behind the lines. Use one or two per icon for the main body.
- Solid: `<path data-tone="solid" d="..."/>`, a closed convex shape filled fully (key tops, knobs, dots).
- One colour: Ovrtone colours each icon itself (text colour, accent when on, white on track tiles).
