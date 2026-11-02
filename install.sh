#!/bin/bash
# Install Husk AI on macOS with one pasted line in Terminal:
#
#   curl -fsSL https://raw.githubusercontent.com/Husk-ai-lab/husk-desktop/main/install.sh | bash
#
# Downloads the latest desktop app (the husk-desktop shell) from the public GitHub releases, checks its SHA-256
# against the digest GitHub computed at upload, copies the app to /Applications (or ~/Applications) and opens it.
# Nothing here is quarantined (curl adds no download flag), so Gatekeeper does not block the unsigned app.
# Built-in macOS tools only (curl, plutil, hdiutil, ditto). No sudo: an admin user can write /Applications.
#
# HUSK_DRY_RUN=1  download, verify and mount only: nothing is copied or opened (for testing).
#
# Everything runs inside main(), called on the last line: if the download of this script is cut short,
# bash never runs a half script. Nothing reads stdin (with `| bash`, stdin is this script).

main() {
    set -euo pipefail

    local REPO="Husk-ai-lab/husk-desktop"
    local API="https://api.github.com/repos/$REPO/releases/latest"
    local IT=0
    if defaults read -g AppleLanguages 2>/dev/null | sed -n 2p | grep -q '"it'; then IT=1; fi

    say() { if [ "$IT" = 1 ]; then echo "$1"; else echo "$2"; fi; }
    fail() { echo; say "❌ $1" "❌ $2"; exit 1; }

    [ "$(uname -s)" = "Darwin" ] || fail "Questo installer è per macOS." "This installer is for macOS."

    local WORK MOUNT=""
    WORK="$(mktemp -d -t husk-install)"
    # shellcheck disable=SC2064  # expand now: the locals are gone when the trap fires
    trap "[ -d '$WORK/mount' ] && hdiutil detach '$WORK/mount' -quiet 2>/dev/null; rm -rf '$WORK'" EXIT

    echo
    say "Installazione di Husk AI" "Installing Husk AI"
    echo "----------------------------------------"

    local TRIPLE
    case "$(uname -m)" in
        arm64) TRIPLE="aarch64-apple-darwin" ;;
        x86_64) TRIPLE="x86_64-apple-darwin" ;;
        *) fail "Processore non supportato: $(uname -m)" "Unsupported processor: $(uname -m)" ;;
    esac

    say "1/4 Cerco l'ultima versione…" "1/4 Looking for the latest version…"
    curl -fsSL --retry 3 -H "Accept: application/vnd.github+json" "$API" -o "$WORK/release.json" \
        || fail "Impossibile contattare GitHub. Controlla la connessione a Internet." \
                "Could not reach GitHub. Check your Internet connection."
    local TAG COUNT NAME="" URL="" DIGEST="" i n
    TAG="$(plutil -extract tag_name raw -o - "$WORK/release.json")"
    COUNT="$(plutil -extract assets raw -o - "$WORK/release.json")"
    for ((i = 0; i < COUNT; i++)); do
        n="$(plutil -extract "assets.$i.name" raw -o - "$WORK/release.json")"
        if [[ "$n" == *"_${TRIPLE}.dmg" ]]; then
            NAME="$n"
            URL="$(plutil -extract "assets.$i.browser_download_url" raw -o - "$WORK/release.json")"
            DIGEST="$(plutil -extract "assets.$i.digest" raw -o - "$WORK/release.json" 2>/dev/null || true)"
            break
        fi
    done
    [ -n "$NAME" ] || fail "Nessun pacchetto per questo Mac nella versione $TAG." "No package for this Mac in release $TAG."
    [[ "$DIGEST" == sha256:* ]] || fail "La versione $TAG non ha un'impronta SHA-256: installazione annullata." \
                                        "Release $TAG has no SHA-256 digest: installation cancelled."
    say "    Versione $TAG ($NAME)" "    Version $TAG ($NAME)"

    say "2/4 Scarico…" "2/4 Downloading…"
    curl -fL --retry 3 --progress-bar "$URL" -o "$WORK/$NAME" || fail "Download non riuscito." "Download failed."
    local GOT
    GOT="$(shasum -a 256 "$WORK/$NAME" | cut -d' ' -f1)"
    [ "sha256:$GOT" = "$DIGEST" ] || fail "Il file scaricato non corrisponde a quello pubblicato: installazione annullata." \
                                          "The downloaded file does not match the published one: installation cancelled."
    say "    Verificato (SHA-256)." "    Verified (SHA-256)."

    say "3/4 Installo…" "3/4 Installing…"
    MOUNT="$WORK/mount"
    mkdir -p "$MOUNT"
    hdiutil attach "$WORK/$NAME" -nobrowse -readonly -quiet -mountpoint "$MOUNT" \
        || fail "Impossibile aprire il pacchetto." "Could not open the package."
    local APP APP_NAME DEST
    APP="$(find "$MOUNT" -maxdepth 1 -name '*.app' -print -quit)"
    [ -n "$APP" ] || fail "Il pacchetto non contiene l'app." "The package does not contain the app."
    APP_NAME="$(basename "$APP")"

    if [ "${HUSK_DRY_RUN:-0}" = 1 ]; then
        say "    Prova a secco: trovato $APP_NAME, niente copiato." "    Dry run: found $APP_NAME, nothing copied."
        return 0
    fi

    if [ -w /Applications ]; then DEST="/Applications"; else DEST="$HOME/Applications"; mkdir -p "$DEST"; fi
    osascript -e "quit app \"${APP_NAME%.app}\"" >/dev/null 2>&1 || true  # an open copy is replaced cleanly
    sleep 1
    rm -rf "${DEST:?}/$APP_NAME"
    ditto "$APP" "$DEST/$APP_NAME" || fail "Copia in $DEST non riuscita." "Copy to $DEST failed."
    xattr -dr com.apple.quarantine "$DEST/$APP_NAME" 2>/dev/null || true
    say "    Installata in $DEST/$APP_NAME" "    Installed in $DEST/$APP_NAME"

    say "4/4 Apro Husk AI…" "4/4 Opening Husk AI…"
    open "$DEST/$APP_NAME"
    echo
    say "✅ Fatto. Al primo avvio Husk scarica i suoi componenti: attendi qualche minuto." \
        "✅ Done. On first start Husk downloads its components: allow a few minutes."
    say "Puoi chiudere il Terminale." "You can close Terminal."
}

main "$@"
