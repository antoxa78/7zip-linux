#!/bin/bash
# Create archive from selected files with format, compression level and encryption options

set -o pipefail

command -v zenity >/dev/null 2>&1 || exit 1
command -v 7z >/dev/null 2>&1 || {
    zenity --error --title="Create Archive" --text="7z is not installed." 2>/dev/null
    exit 1
}

TIMESTAMP=$(date +"%Y-%m-%d_%H-%M-%S")
SEPARATOR=$'\x1f'

center_window() {
    local target_title="$1"
    command -v wmctrl >/dev/null 2>&1 || return 0

    local window_id=""
    local window_width=0
    local window_height=0
    local work_x=0
    local work_y=0
    local work_width=0
    local work_height=0
    local attempt
    local id desktop x y width height title

    for attempt in $(seq 1 40); do
        while read -r id desktop x y width height title; do
            case "$title" in
                *"$target_title"*)
                    window_id="$id"
                    window_width="$width"
                    window_height="$height"
                    break
                    ;;
            esac
        done <<< "$(wmctrl -lG 2>/dev/null)"

        [ -n "$window_id" ] && break
        sleep 0.05
    done

    [ -n "$window_id" ] || return 0

    read -r work_x work_y work_width work_height <<< "$(wmctrl -d 2>/dev/null \
        | sed -n 's/.*WA: \(-*[0-9][0-9]*\),\(-*[0-9][0-9]*\) \([0-9][0-9]*\)x\([0-9][0-9]*\).*/\1 \2 \3 \4/p' \
        | head -1)"
    [ "${work_width:-0}" -gt 0 ] || return 0
    [ "${work_height:-0}" -gt 0 ] || return 0

    local left=$(( work_x + (work_width - window_width) / 2 ))
    local top=$(( work_y + (work_height - window_height) / 2 ))
    wmctrl -i -r "$window_id" -e "0,$left,$top,-1,-1" 2>/dev/null || true
    sleep 0.1
    wmctrl -i -r "$window_id" -e "0,$left,$top,-1,-1" 2>/dev/null || true
}

run_centered_question() {
    local title="$1"
    local text="$2"
    local ok_label="$3"
    local cancel_label="$4"
    local dialog_pid

    zenity --question --title="$title" --text="$text" \
        --ok-label="$ok_label" --cancel-label="$cancel_label" \
        >/dev/null 2>/dev/null &
    dialog_pid=$!
    center_window "$title"
    wait "$dialog_pid"
}

run_centered_message() {
    local kind="$1"
    local title="$2"
    local text="$3"
    local dialog_pid

    zenity "$kind" --title="$title" --text="$text" \
        >/dev/null 2>/dev/null &
    dialog_pid=$!
    center_window "$title"
    wait "$dialog_pid"
}

CHOICE=$(zenity --forms --title="Create Archive" --width=480 \
    --text="Create archive from selected files" \
    --add-entry="Archive name (without extension):" \
    --add-combo="Format:" --combo-values="7z|zip|tar|tar.gz|tar.bz2|tar.xz|tar.zst" \
    --add-combo="Compression level:" --combo-values="Normal|Store|Fastest|Fast|Maximum|Ultra" \
    --add-password="Password (optional):" \
    --add-combo="Encryption:" --combo-values="None|Encrypt contents|Encrypt contents + file names" \
    --separator="$SEPARATOR")
[ -z "$CHOICE" ] && exit 0

IFS="$SEPARATOR" read -r NAME FORMAT LEVEL PASSWORD ENCRYPTION <<< "$CHOICE"
[ -n "$NAME" ] || NAME="archive_${TIMESTAMP}"

case "$FORMAT" in
    7z)      EXT="7z";     CTYPE="-t7z";   SPLIT="" ;;
    zip)     EXT="zip";    CTYPE="-tzip";  SPLIT="" ;;
    tar)     EXT="tar";    CTYPE="-ttar";  SPLIT="" ;;
    tar.gz)  EXT="tar.gz"; CTYPE="targz";  SPLIT="-tgzip" ;;
    tar.bz2) EXT="tar.bz2"; CTYPE="tarbz2"; SPLIT="-tbzip2" ;;
    tar.xz)  EXT="tar.xz"; CTYPE="tarxz";  SPLIT="-txz" ;;
    tar.zst) EXT="tar.zst"; CTYPE="tarzst"; SPLIT="-tzstd" ;;
    *)       EXT="7z";     CTYPE="-t7z";   SPLIT="" ;;
esac

case "$NAME" in
    *."$EXT") ;;
    *) NAME="$NAME.$EXT" ;;
esac

if [ -e "$NAME" ]; then
    run_centered_question "Create Archive" \
        "The archive already exists:\n$NAME\n\nOverwrite it?" \
        "Overwrite" "Cancel" || exit 0
fi

case "$LEVEL" in
    Store)   MX=0 ;;
    Fastest) MX=1 ;;
    Fast)    MX=2 ;;
    Normal)  MX=3 ;;
    Maximum) MX=5 ;;
    Ultra)   MX=7 ;;
    *)       MX=3 ;;
esac

ENCRYPT="FALSE"
case "$ENCRYPTION" in
    "Encrypt contents + file names") ENCRYPT="TRUE" ;;
esac

PASS_ARGS=()
if [ -n "$PASSWORD" ]; then
    case "$CTYPE" in
        -t7z|-tzip)
            PASS_ARGS=("-p$PASSWORD")
            if [ "$ENCRYPT" = "TRUE" ] && [ "$CTYPE" = "-t7z" ]; then
                PASS_ARGS+=("-mhe=on")
            fi
            ;;
        *)
            run_centered_message error "Create Archive" \
                "tar archives cannot be encrypted. Choose 7z or zip format for password protection."
            exit 1
            ;;
    esac
fi

process_running() {
    kill -0 "$1" 2>/dev/null || return 1
    case "$(ps -o stat= -p "$1" 2>/dev/null)" in
        Z*) return 1 ;;
    esac
    return 0
}

progress_feed() {
    local log="$1"
    local pid="$2"
    local button="$3"
    local last=""
    local pct
    local label="Compressing"

    [ "$button" = "Resume" ] && label="Paused at"

    while process_running "$pid"; do
        pct=$(grep -oE '[0-9]+%' "$log" 2>/dev/null | tail -1 | tr -d '%')
        if [ -n "$pct" ] && [ "$pct" != "$last" ]; then
            printf '# %s %s%%\n%s\n' "$label" "$pct" "$pct"
            last="$pct"
        fi
        sleep 0.2
    done

    pct=$(grep -oE '[0-9]+%' "$log" 2>/dev/null | tail -1 | tr -d '%')
    if [ -n "$pct" ] && [ "$pct" != "$last" ]; then
        printf '# %s %s%%\n%s\n' "$label" "$pct" "$pct"
    fi
    printf '100\n'
}

show_progress() {
    local log="$1"
    local pid="$2"
    local fifo="$3"
    local button="$4"
    local feed_pid
    local zenity_pid
    local response_file="$fifo.response"
    local response
    local status

    progress_feed "$log" "$pid" "$button" > "$fifo" &
    feed_pid=$!
    zenity --progress --title="Creating Archive" --modal \
        --auto-close --extra-button="$button" --cancel-label="Cancel" \
        --width=350 < "$fifo" > "$response_file" &
    zenity_pid=$!
    center_window "Creating Archive"
    wait "$zenity_pid"
    status=$?
    response=$(cat "$response_file" 2>/dev/null)
    kill "$feed_pid" 2>/dev/null || true
    wait "$feed_pid" 2>/dev/null || true
    rm -f "$response_file"
    PROGRESS_RESPONSE="$response"
    return "$status"
}

compress() {
    local progress_dir
    local log
    local fifo
    local pid
    local response
    local status
    local paused="FALSE"

    progress_dir=$(mktemp -d) || return 1
    log="$progress_dir/7z.log"
    fifo="$progress_dir/progress.fifo"
    : > "$log"
    mkfifo "$fifo" || { rm -rf "$progress_dir"; return 1; }

    7z a -bsp1 "$@" > "$log" 2>&1 &
    pid=$!

    while process_running "$pid"; do
        if [ "$paused" = "TRUE" ]; then
            show_progress "$log" "$pid" "$fifo" "Resume"
        else
            show_progress "$log" "$pid" "$fifo" "Pause"
        fi
        status=$?
        response="$PROGRESS_RESPONSE"

        if [ "$response" = "Pause" ] && [ "$paused" = "FALSE" ]; then
            kill -STOP "$pid" 2>/dev/null || true
            paused="TRUE"
            continue
        fi

        if [ "$response" = "Resume" ] && [ "$paused" = "TRUE" ]; then
            kill -CONT "$pid" 2>/dev/null || true
            paused="FALSE"
            continue
        fi

        if [ "$status" -ne 0 ]; then
            if run_centered_question "Cancel Archive" \
                "Do you want to cancel archive creation?" \
                "Cancel Archive" "Continue"; then
                kill -CONT "$pid" 2>/dev/null || true
                kill -TERM "$pid" 2>/dev/null || true
                wait "$pid" 2>/dev/null || true
                rm -rf "$progress_dir"
                return 125
            fi
            continue
        fi
        break
    done

    if [ "$paused" = "TRUE" ]; then
        kill -CONT "$pid" 2>/dev/null || true
    fi
    wait "$pid"
    status=$?
    rm -rf "$progress_dir"
    return "$status"
}

fail() {
    run_centered_message error "Create Archive" "$1"
    exit 1
}

if [ "$ENCRYPTION" != "None" ] && [ -z "$PASSWORD" ]; then
    fail "Enter a password when encryption is selected."
fi

if [ "$ENCRYPTION" = "Encrypt contents + file names" ] && [ "$CTYPE" != "-t7z" ]; then
    fail "Only the 7z format supports encrypting file names."
fi

TMPTAR=""
cleanup() {
    [ -z "$TMPTAR" ] || rm -f "$TMPTAR"
}
trap cleanup EXIT

if [ "$CTYPE" = "targz" ] || [ "$CTYPE" = "tarbz2" ] || [ "$CTYPE" = "tarxz" ] || [ "$CTYPE" = "tarzst" ]; then
    TMPTAR="$(mktemp "${NAME%.*}.XXXXXX")" || fail "Could not create temporary file."
    rm -f "$TMPTAR"
    TMPTAR="$TMPTAR.tar"
    compress -ttar "$TMPTAR" "$@" || fail "Failed to create tar archive."
    compress "$SPLIT" "-mx=$MX" "$NAME" "$TMPTAR" || fail "Failed to compress archive."
    rm -f "$TMPTAR"
    TMPTAR=""
else
    compress "$CTYPE" "-mx=$MX" "${PASS_ARGS[@]}" "$NAME" "$@" || fail "Failed to create archive."
fi

run_centered_message info "Create Archive" "Archive created: $NAME"
