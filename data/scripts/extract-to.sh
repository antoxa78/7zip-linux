#!/bin/bash
# Extract archive(s) to chosen folder
DIR=$(zenity --file-selection --directory --title="Extract to folder")
if [ -n "$DIR" ]; then
    for f in "$@"; do
        7z x "$f" -o"$DIR" -aoa
    done
fi