#!/bin/bash
# Extract archive(s) here using 7-Zip
for f in "$@"; do
    7z x "$f" -o"$(dirname "$f")" -aoa
done