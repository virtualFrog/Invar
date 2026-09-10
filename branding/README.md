# Branding

`icon.svg` is the master. Everything under `src-tauri/icons/` is generated from
it and should never be edited by hand.

## The mark

A teal key column with three records beside it. The column doubles as the I of
Invar; the last record is short because real inventory is ragged.

It replaced a blue shield over circuit traces with the letters "VM" inside it.
That icon was wrong twice over: a security metaphor for a tool that does no
security, and detail that turned to mush below 64px. An app icon is read at
16px in a sidebar far more often than at 512px in a dock, so this one is four
rectangles and two tones and nothing else.

Colours come from `src/styles.css`: the graphite surface and the single teal
accent, deliberately not a vendor blue.

## Regenerating the icon set

There is no SVG rasterizer in this toolchain, so the PNG is rendered with
headless Chrome and the set is generated from that.

```bash
# 1. SVG to a 1024x1024 PNG with a transparent background
cat > /tmp/master.html <<'HTML'
<style>html,body{margin:0;padding:0;width:1024px;height:1024px;background:transparent;overflow:hidden}
img{display:block;width:1024px;height:1024px}</style>
<img src="file:///absolute/path/to/branding/icon.svg">
HTML

"/Applications/Google Chrome.app/Contents/MacOS/Google Chrome" \
  --headless=new --disable-gpu --hide-scrollbars --allow-file-access-from-files \
  --default-background-color=00000000 --window-size=1024,1024 \
  --screenshot=branding/icon-1024.png file:///tmp/master.html

# 2. Every platform size, plus .icns and .ico
npm run tauri icon -- branding/icon-1024.png

# 3. This is a desktop app. Drop the mobile sets the generator adds anyway.
rm -rf src-tauri/icons/android src-tauri/icons/ios
```

Then check `src-tauri/icons/32x32.png` before committing. If the mark is not
legible there, the mark is wrong, not the export.

## The in-app mark

`src/index.html` carries the same shape without its tile, as inline SVG at 18px.
It is kept in step with `icon.svg` by hand, which is cheap for four rectangles
and avoids a build step for one icon.
