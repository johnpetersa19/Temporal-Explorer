#!/bin/bash
# po/update-pot.sh
#
# Regenerates po/temporal-explorer.pot by extracting translatable strings
# from ALL sources: Rust (.rs), Blueprint UI (.blp) and native UI (.ui).
# Syncs LINGUAS <-> .po files and runs msgmerge on every existing .po file.
#
# ────────────────────────────────────────────────────────────────────────────
# HOW IT WORKS
# ────────────────────────────────────────────────────────────────────────────
#
#  Step 1 — Rust source scan (.rs with gettext())
#  Step 2 — Blueprint scan (.blp) via grep lookbehind: _("..") pattern
#  Step 3 — Native UI scan (.ui with translatable="yes") via xgettext Glade
#  Step 4 — Merge partial .pot files via Python (no msgcat dependency)
#  Step 5 — POTFILES.in regeneration
#  Step 6 — Bidirectional LINGUAS <-> .po sync + msgmerge
#
# USAGE
#   cd <repo root>
#   bash po/update-pot.sh

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/.."
PO_DIR="$ROOT/po"
OUT="$PO_DIR/temporal-explorer.pot"
POTFILES="$PO_DIR/POTFILES.in"
LINGUAS_FILE="$PO_DIR/LINGUAS"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

echo "=== Temporal Explorer — regenerating .pot ==="
echo ""

PKG_VER=$(grep -m1 '^version' "$ROOT/Cargo.toml" | sed 's/.*= *"//;s/"//')
DATE=$(date +"%Y-%m-%d %H:%M%z")

# ── 1. Rust files ───────────────────────────────────────────────────────────────
echo "[1/6] Scanning Rust files (.rs with gettext())..."
mapfile -t RUST_FILES < <(
    find "$ROOT/src" -name "*.rs" -type f \
    | xargs grep -l 'gettext(' 2>/dev/null \
    | sort
)
echo "   → ${#RUST_FILES[@]} .rs files found"

if [[ ${#RUST_FILES[@]} -gt 0 ]]; then
    xgettext \
        --from-code=UTF-8 \
        --language=C \
        --keyword=gettext \
        --add-comments=translators \
        --package-name=temporal-explorer \
        --package-version="$PKG_VER" \
        --output="$TMP/rust.pot" \
        "${RUST_FILES[@]}" 2>/dev/null
    RS_COUNT=$(grep -c '^msgid ' "$TMP/rust.pot" 2>/dev/null || echo 0)
    echo "   → rust.pot: $RS_COUNT entries"
else
    touch "$TMP/rust.pot"
fi

# ── 2. Blueprint files (.blp) ───────────────────────────────────────────────────
echo "[2/6] Scanning Blueprint files (.blp with _(\"...\"))..."
mapfile -t BLP_FILES < <(
    find "$ROOT/src" -name "*.blp" -type f \
    | xargs grep -l '_("' 2>/dev/null \
    | sort
)
echo "   → ${#BLP_FILES[@]} .blp files found"

python3 - "$ROOT" "${BLP_FILES[@]}" > "$TMP/blp.entries" << 'PYEOF'
import re
import sys
from pathlib import Path

root = Path(sys.argv[1])
files = [Path(p) for p in sys.argv[2:]]

def po_escape(s: str) -> str:
    return (
        s.replace("\\", "\\\\")
         .replace('"', '\\"')
         .replace("\t", "\\t")
         .replace("\r", "\\r")
         .replace("\n", "\\n")
    )

# Match Blueprint _("...") strings, including escaped quotes/backslashes.
pattern = re.compile(r'_\("((?:\\.|[^"\\])*)"\)')

for blp in files:
    rel = blp.relative_to(root).as_posix()
    content = blp.read_text(encoding="utf-8", errors="replace")

    for match in pattern.finditer(content):
        raw = match.group(1)

        # Decode Blueprint-style escapes, then re-escape for PO syntax.
        try:
            decoded = bytes(raw, "utf-8").decode("unicode_escape")
        except UnicodeDecodeError:
            decoded = raw

        print(f"#: {rel}")
        print(f'msgid "{po_escape(decoded)}"')
        print('msgstr ""')
        print()
PYEOF
BLP_COUNT=$(grep -c '^msgid ' "$TMP/blp.entries" 2>/dev/null || echo 0)
echo "   → blp.entries: $BLP_COUNT entries"

# ── 3. Native UI files (.ui with translatable="yes") ────────────────────────
echo "[3/6] Scanning native UI files (.ui with translatable=\"yes\")..."
mapfile -t UI_FILES < <(
    find "$ROOT/src" -name "*.ui" -type f \
    | xargs grep -l 'translatable="yes"' 2>/dev/null \
    | sort
)
echo "   → ${#UI_FILES[@]} .ui files found"

if [[ ${#UI_FILES[@]} -gt 0 ]]; then
    xgettext \
        --from-code=UTF-8 \
        --language=Glade \
        --add-comments=translators \
        --package-name=temporal-explorer \
        --package-version="$PKG_VER" \
        --output="$TMP/ui.pot" \
        "${UI_FILES[@]}" 2>/dev/null || true
fi
[[ -f "$TMP/ui.pot" ]] || touch "$TMP/ui.pot"
UI_COUNT=$(grep -c '^msgid ' "$TMP/ui.pot" 2>/dev/null || echo 0)
echo "   → ui.pot: $UI_COUNT entries"

# ── 4. Merge via Python (sem dependencia de msgcat) ────────────────────────────
echo "[4/6] Merging rust.pot + blp.entries + ui.pot..."
python3 - "$TMP/rust.pot" "$TMP/blp.entries" "$TMP/ui.pot" "$OUT" \
         "$PKG_VER" "$DATE" << 'PYEOF'
import sys, re, os

rust_pot, blp_entries, ui_pot, out_path, pkg_ver, date = sys.argv[1:]

def extract_entries(path):
    """Return list of (refs, msgid) from a .pot/.entries file, skip header."""
    entries = []
    if not os.path.exists(path) or os.path.getsize(path) == 0:
        return entries
    with open(path, encoding='utf-8') as f:
        content = f.read()
    # Split on blank lines
    blocks = re.split(r'\n{2,}', content.strip())
    for block in blocks:
        lines = block.strip().splitlines()
        msgid_match = re.search(r'^msgid "(.+)"$', block, re.MULTILINE)
        if not msgid_match:
            continue  # skip header block (msgid "")
        msgid = msgid_match.group(1)
        refs = [l.strip() for l in lines if l.startswith('#:')]
        entries.append((refs, msgid))
    return entries

def po_escape(s):
    return (
        s.replace("\\", "\\\\")
         .replace('"', '\\"')
         .replace("\t", "\\t")
         .replace("\r", "\\r")
         .replace("\n", "\\n")
    )

seen = {}
ordered = []

for path in [rust_pot, blp_entries, ui_pot]:
    for refs, msgid in extract_entries(path):
        if msgid not in seen:
            seen[msgid] = refs
            ordered.append(msgid)
        else:
            seen[msgid] = seen[msgid] + refs

with open(out_path, 'w', encoding='utf-8') as f:
    f.write('msgid ""\n')
    f.write('msgstr ""\n')
    f.write(f'"Project-Id-Version: temporal-explorer {pkg_ver}\\n"\n')
    f.write(f'"POT-Creation-Date: {date}\\n"\n')
    f.write('"PO-Revision-Date: YEAR-MO-DA HO:MI+ZONE\\n"\n')
    f.write('"Last-Translator: FULL NAME <EMAIL@ADDRESS>\\n"\n')
    f.write('"Language-Team: LANGUAGE <LL@li.org>\\n"\n')
    f.write('"Language: \\n"\n')
    f.write('"MIME-Version: 1.0\\n"\n')
    f.write('"Content-Type: text/plain; charset=UTF-8\\n"\n')
    f.write('"Content-Transfer-Encoding: 8bit\\n"\n')
    f.write('\n')
    for msgid in ordered:
        refs = seen[msgid]
        for r in refs:
            f.write(f'{r}\n')
        f.write(f'msgid "{po_escape(msgid)}"\n')
        f.write('msgstr ""\n')
        f.write('\n')
print(f'   → {out_path} written ({len(ordered)} total entries)')
PYEOF

TOTAL=$(grep -c '^msgid ' "$OUT" 2>/dev/null || echo 0)

# ── 5. Update POTFILES.in ────────────────────────────────────────────────────────
echo "[5/6] Updating POTFILES.in..."
{
    echo "# Auto-generated by po/update-pot.sh — do not edit manually"
    echo ""
    for f in "${RUST_FILES[@]}"; do echo "${f#"$ROOT/"}"; done
    for f in "${BLP_FILES[@]}";  do echo "${f#"$ROOT/"}"; done
    for f in "${UI_FILES[@]}";   do echo "${f#"$ROOT/"}"; done
} | grep -v '^#\|^$' | sort -u > "$TMP/pf_sorted"
{
    echo "# Auto-generated by po/update-pot.sh — do not edit manually"
    echo ""
    cat "$TMP/pf_sorted"
} > "$POTFILES"
PF_COUNT=$(grep -c '^src/' "$POTFILES" 2>/dev/null || echo 0)
echo "   → POTFILES.in updated ($PF_COUNT files)"

# ── 6. Sync LINGUAS <-> .po + msgmerge ─────────────────────────────────────────
echo "[6/6] Syncing LINGUAS <-> .po files..."

declare -A LINGUAS_SET
while IFS= read -r lang || [[ -n "$lang" ]]; do
    lang="$(echo "$lang" | tr -d '[:space:]')"
    [[ -z "$lang" || "$lang" == \#* ]] && continue
    LINGUAS_SET["$lang"]=1
done < "$LINGUAS_FILE"

ADDED_TO_LINGUAS=()
CREATED_PO=()

for po_file in "$PO_DIR"/*.po; do
    [[ -f "$po_file" ]] || continue
    lang=$(basename "$po_file" .po)
    if [[ -z "${LINGUAS_SET[$lang]:-}" ]]; then
        echo "   + LINGUAS: adding '$lang'"
        echo "$lang" >> "$LINGUAS_FILE"
        LINGUAS_SET["$lang"]=1
        ADDED_TO_LINGUAS+=("$lang")
    fi
done

for lang in "${!LINGUAS_SET[@]}"; do
    po_file="$PO_DIR/$lang.po"
    if [[ ! -f "$po_file" ]]; then
        echo "   + Creating $lang.po via msginit..."
        msginit \
            --input="$OUT" \
            --locale="$lang" \
            --output="$po_file" \
            --no-translator 2>/dev/null || true
        [[ -f "$po_file" ]] && CREATED_PO+=("$lang") || \
            echo "   ⚠ msginit failed for '$lang'"
    fi
done

sort -u "$LINGUAS_FILE" > "$TMP/linguas_sorted"
mv "$TMP/linguas_sorted" "$LINGUAS_FILE"

[[ ${#ADDED_TO_LINGUAS[@]} -gt 0 ]] && echo "   → Added to LINGUAS: ${ADDED_TO_LINGUAS[*]}"
[[ ${#CREATED_PO[@]}       -gt 0 ]] && echo "   → .po files created: ${CREATED_PO[*]}"
[[ ${#ADDED_TO_LINGUAS[@]} -eq 0 && ${#CREATED_PO[@]} -eq 0 ]] && \
    echo "   → LINGUAS and .po files already in sync"

echo ""
echo "✓ Generated : $OUT"
echo "✓ Entries   : $TOTAL"
echo "✓ Version   : temporal-explorer $PKG_VER"
echo "✓ Languages : $(tr '\n' ' ' < "$LINGUAS_FILE" | xargs)"

# ── Update ALL .po files ───────────────────────────────────────────────────────────
echo ""
echo "=== Updating all .po files with msgmerge ==="
for po in "$PO_DIR"/*.po; do
    [[ -f "$po" ]] || continue
    lang=$(basename "$po" .po)
    printf "  → %-14s" "$lang.po"
    msgmerge --quiet --update --backup=none "$po" "$OUT"
    UNTRANSLATED=$(grep -c '^msgstr ""' "$po" 2>/dev/null || echo 0)
    TOTAL_ENTRIES=$(grep -c '^msgid '   "$po" 2>/dev/null || echo 0)
    echo " ($UNTRANSLATED/$TOTAL_ENTRIES untranslated)"
done
echo "✓ All .po files updated!"
