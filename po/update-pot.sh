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

# ── Dependency check ─────────────────────────────────────────────────────────
REQUIRED_CMDS=(
    date
    grep
    mktemp
    python3
    sed
    sort
    xgettext
    msginit
    msgmerge
)

for cmd in "${REQUIRED_CMDS[@]}"; do
    if ! command -v "$cmd" >/dev/null 2>&1; then
        echo "Error: required command not found: $cmd" >&2
        exit 1
    fi
done

mkdir -p "$PO_DIR"
[[ -f "$LINGUAS_FILE" ]] || touch "$LINGUAS_FILE"

echo "=== Temporal Explorer — regenerating .pot ==="
echo ""

PKG_VER=$(grep -m1 '^version' "$ROOT/Cargo.toml" | sed 's/.*= *"//;s/"//')
DATE=$(date +"%Y-%m-%d %H:%M%z")

# ── Helpers ─────────────────────────────────────────────────────────────────
count_po_entries() {
    local file="$1"
    local count

    [[ -s "$file" ]] || {
        echo 0
        return
    }

    # Every PO entry has exactly one `msgid` directive, including entries
    # whose value continues across multiple quoted lines. Exclude the header.
    count=$(grep -c '^msgid ' "$file" || true)
    (( count > 0 )) && count=$((count - 1))
    echo "$count"
}

clean_linguas_file() {
    local file="$1"

    [[ -f "$file" ]] || return 0

    sed -e 's/#.*//' \
        -e 's/^[[:space:]]*//;s/[[:space:]]*$//' \
        -e '/^$/d' "$file" \
        | LC_ALL=C sort -u
}

write_sorted_linguas() {
    local file="$1"
    shift

    {
        echo "# Please keep this file sorted alphabetically."
        echo ""
        printf '%s\n' "$@" | sed '/^$/d' | LC_ALL=C sort -u
    } > "$file"
}

# Discover and classify all translatable sources in a single filesystem walk.
# NUL-delimited records preserve paths containing spaces or newlines.
RUST_FILES=()
BLP_FILES=()
UI_FILES=()

while IFS= read -r -d '' kind && IFS= read -r -d '' file; do
    case "$kind" in
        rust) RUST_FILES+=("$file") ;;
        blp)  BLP_FILES+=("$file") ;;
        ui)   UI_FILES+=("$file") ;;
    esac
done < <(python3 - "$ROOT/src" << 'PYEOF'
import sys
from pathlib import Path

source_dir = Path(sys.argv[1])
patterns = {
    ".rs": ("rust", "gettext("),
    ".blp": ("blp", '_("'),
    ".ui": ("ui", 'translatable="yes"'),
}

for path in sorted(source_dir.rglob("*")):
    match = patterns.get(path.suffix)
    if match is None or not path.is_file():
        continue

    kind, marker = match
    content = path.read_text(encoding="utf-8", errors="replace")
    if marker in content:
        sys.stdout.buffer.write(kind.encode() + b"\0")
        sys.stdout.buffer.write(str(path).encode() + b"\0")
PYEOF
)

# ── 1. Rust files ───────────────────────────────────────────────────────────────
echo "[1/6] Scanning Rust files (.rs with gettext())..."
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
    RS_COUNT=$(count_po_entries "$TMP/rust.pot")
    echo "   → rust.pot: $RS_COUNT entries"
else
    touch "$TMP/rust.pot"
fi

# ── 2. Blueprint files (.blp) ───────────────────────────────────────────────────
echo "[2/6] Scanning Blueprint files (.blp with _(\"...\"))..."
echo "   → ${#BLP_FILES[@]} .blp files found"

python3 - "$ROOT" "${BLP_FILES[@]}" > "$TMP/blp.entries" << 'PYEOF'
import ast
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
            decoded = ast.literal_eval(f'"{raw}"')
        except Exception:
            decoded = raw

        line_no = content.count("\n", 0, match.start()) + 1

        print(f"#: {rel}:{line_no}")
        print(f'msgid "{po_escape(decoded)}"')
        print('msgstr ""')
        print()
PYEOF
BLP_COUNT=$(count_po_entries "$TMP/blp.entries")
echo "   → blp.entries: $BLP_COUNT entries"

# ── 3. Native UI files (.ui with translatable="yes") ────────────────────────
echo "[3/6] Scanning native UI files (.ui with translatable=\"yes\")..."
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
UI_COUNT=$(count_po_entries "$TMP/ui.pot")
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

TOTAL=$(count_po_entries "$OUT")

# ── 5. Update POTFILES.in ────────────────────────────────────────────────────────
echo "[5/6] Updating POTFILES.in..."
{
    for f in "${RUST_FILES[@]}"; do echo "${f#"$ROOT/"}"; done
    for f in "${BLP_FILES[@]}";  do echo "${f#"$ROOT/"}"; done
    for f in "${UI_FILES[@]}";   do echo "${f#"$ROOT/"}"; done
} | sort -u > "$TMP/pf_sorted"
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

mapfile -t CLEAN_LANGS < <(clean_linguas_file "$LINGUAS_FILE")

for lang in "${CLEAN_LANGS[@]}"; do
    LINGUAS_SET["$lang"]=1
done

ADDED_TO_LINGUAS=()
CREATED_PO=()

for po_file in "$PO_DIR"/*.po; do
    [[ -f "$po_file" ]] || continue
    lang=$(basename "$po_file" .po)
    if [[ -z "${LINGUAS_SET[$lang]:-}" ]]; then
        echo "   + LINGUAS: adding '$lang'"
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

mapfile -t FINAL_LANGS < <(
    printf '%s\n' "${!LINGUAS_SET[@]}" | LC_ALL=C sort -u
)

write_sorted_linguas "$LINGUAS_FILE" "${FINAL_LANGS[@]}"

[[ ${#ADDED_TO_LINGUAS[@]} -gt 0 ]] && echo "   → Added to LINGUAS: ${ADDED_TO_LINGUAS[*]}"
[[ ${#CREATED_PO[@]}       -gt 0 ]] && echo "   → .po files created: ${CREATED_PO[*]}"
[[ ${#ADDED_TO_LINGUAS[@]} -eq 0 && ${#CREATED_PO[@]} -eq 0 ]] && \
    echo "   → LINGUAS and .po files already in sync"

echo ""
echo "✓ Generated : $OUT"
echo "✓ Entries   : $TOTAL"
echo "✓ Version   : temporal-explorer $PKG_VER"
LANGUAGE_LIST="${FINAL_LANGS[*]}"
echo "✓ Languages : $LANGUAGE_LIST"

# ── Update ALL .po files ───────────────────────────────────────────────────────────
echo ""
echo "=== Updating all .po files with msgmerge ==="
for po in "$PO_DIR"/*.po; do
    [[ -f "$po" ]] || continue
    lang=$(basename "$po" .po)
    printf "  → %-14s" "$lang.po"
    msgmerge --quiet --update --backup=none "$po" "$OUT"
    UNTRANSLATED=$(grep -c '^msgstr ""' "$po" 2>/dev/null || echo 0)
    TOTAL_ENTRIES=$(count_po_entries "$po")

    # Remove PO header from untranslated count.
    if [[ "$UNTRANSLATED" -gt 0 ]]; then
        UNTRANSLATED=$((UNTRANSLATED - 1))
    fi

    echo " ($UNTRANSLATED/$TOTAL_ENTRIES untranslated)"
done
echo "✓ All .po files updated!"
