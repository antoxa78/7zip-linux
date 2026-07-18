#!/bin/bash
# Create 7z archive from selected files with progress
TIMESTAMP=$(date +"%Y-%m-%d_%H-%M-%S")
ARCHIVE=$(zenity --entry --title="Create Archive" --text="Archive name:" --entry-text="archive_${TIMESTAMP}.7z")
if [ -n "$ARCHIVE" ]; then
    7z a -bsp1 "$ARCHIVE" "$@" 2>/dev/null | sed -u 's/\r/\n/g' | while IFS= read -r line; do
        PCT=$(echo "$line" | grep -oP '\d+(?=%)' | tail -1)
        if [ -n "$PCT" ]; then
            echo "# Compressing... ${PCT}%"
            echo "$PCT"
        fi
    done | zenity --progress --title="Creating Archive" --auto-close --no-cancel --width=350
    zenity --info --text="Archive created: $ARCHIVE" 2>/dev/null
fi
