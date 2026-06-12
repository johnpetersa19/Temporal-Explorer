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
#  Step 4 — Merge all partial .pot files with msgcat
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
    printf 'msgid ""\nmsgstr ""\n"Content-Type: text/plain; charset=UTF-8\\n"\n' > "$TMP/rust.pot"
fi

# ── 2. Blueprint files (.blp) ───────────────────────────────────────────────────
echo "[2/6] Scanning Blueprint files (.blp with _(\"...\"))..."
mapfile -t BLP_FILES < <(
    find "$ROOT/src" -name "*.blp" -type f \
    | xargs grep -l '_("' 2>/dev/null \
    | sort
)
echo "   → ${#BLP_FILES[@]} .blp files found"

# Blueprint syntax uses _("string") — not XML, so xgettext Glade cannot parse it.
# Extract with lookbehind regex: match content inside _("...")
{
    printf 'msgid ""\nmsgstr ""\n'
    printf '"Content-Type: text/plain; charset=UTF-8\\n"\n'
    printf '"Content-Transfer-Encoding: 8bit\\n"\n\n'
    for blp in "${BLP_FILES[@]}"; do
        rel="${blp#"$ROOT/"}"
        grep -oP '(?<=_\(")[^"]+(?=")' "$blp" 2>/dev/null \
        | while IFS= read -r str; do
            printf '#: %s\n' "$rel"
            printf 'msgid "%s"\n' "$str"
            printf 'msgstr ""\n\n'
        done
    done
} > "$TMP/blp.pot"
BLP_COUNT=$(grep -c '^msgid ' "$TMP/blp.pot" 2>/dev/null || echo 0)
echo "   → blp.pot: $BLP_COUNT entries"

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
    UI_COUNT=$(grep -c '^msgid ' "$TMP/ui.pot" 2>/dev/null || echo 0)
    echo "   → ui.pot: $UI_COUNT entries"
else
    printf 'msgid ""\nmsgstr ""\n"Content-Type: text/plain; charset=UTF-8\\n"\n' > "$TMP/ui.pot"
fi

# ── 4. Merge rust.pot + blp.pot + ui.pot ────────────────────────────────────────
echo "[4/6] Merging rust.pot + blp.pot + ui.pot..."
msgcat \
    --use-first \
    --output="$TMP/merged.pot" \
    "$TMP/rust.pot" \
    "$TMP/blp.pot" \
    "$TMP/ui.pot" 2>/dev/null

sed \
    -e "s|^\"Project-Id-Version:.*|\"Project-Id-Version: temporal-explorer $PKG_VER\\\\n\"|" \
    -e "s|^\"POT-Creation-Date:.*|\"POT-Creation-Date: $DATE\\\\n\"|" \
    -e "s|^\"PO-Revision-Date:.*|\"PO-Revision-Date: YEAR-MO-DA HO:MI+ZONE\\\\n\"|" \
    -e "s|^\"Last-Translator:.*|\"Last-Translator: FULL NAME <EMAIL@ADDRESS>\\\\n\"|" \
    -e "s|^\"Language-Team:.*|\"Language-Team: LANGUAGE <LL@li.org>\\\\n\"|" \
    -e "s|^\"Language:.*|\"Language: \\\\n\"|" \
    "$TMP/merged.pot" > "$OUT"

TOTAL=$(grep -c '^msgid ' "$OUT" 2>/dev/null || echo 0)
echo "   → $OUT written ($TOTAL total entries)"

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
