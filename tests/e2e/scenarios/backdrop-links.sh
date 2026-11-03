# backdrop:// links: an unknown ID opens the "link a video" dialog; a linked
# ID plays the linked library item on every display.
S=$T/shots
mkdir -p "$T/cfg/frontrise"
cat > "$T/cfg/frontrise/config.json" <<JSON
{
  "library": [
    {"id": "bars", "title": "Test bars", "kind": "video", "path": "$ROOT/samples/test-bars.webm",
     "backdrop_id": "F7A4D5BB-1A0D-424B-AF58-F7BAE32A3D75"}
  ],
  "displays": {"Meta-0": {"source": {"type": "off"}}, "Meta-1": {"source": {"type": "off"}}}
}
JSON
"$FR" --background >> "$T/app.log" 2>&1 &
sleep 3
cmd o1 "overview hide"
"$FR" "backdrop://wallpaper/00000000-1111-2222-3333-444444444444" >> "$T/app.log" 2>&1
sleep 3
cmd s1 "shot $S/link-unknown.png"
"$FR" "backdrop://wallpaper/f7a4d5bb-1a0d-424b-af58-f7bae32a3d75" >> "$T/app.log" 2>&1
sleep 3
cmd s2 "shot $S/link-known.png"
echo "== displays: $(python3 -I -c "import json,sys; print(json.load(open(sys.argv[1]))['displays'])" "$T/cfg/frontrise/config.json")" >> "$T/result.txt"
