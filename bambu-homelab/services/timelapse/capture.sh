#!/bin/bash
# Timelapse capture: grabs a frame from the RTSP stream every N seconds
# during a print. When the print finishes, stitches frames into an mp4.
#
# Controlled via files in /timelapses/{printer_id}/:
#   - .active     = capture is running (created by API, removed on stop)
#   - frames/     = captured PNG frames
#   - output.mp4  = final timelapse video

set -e

TIMELAPSES_DIR=/timelapses
INTERVAL=${TIMELAPSE_INTERVAL:-10}  # seconds between captures

mkdir -p "$TIMELAPSES_DIR"

echo "Timelapse service ready. Interval: ${INTERVAL}s"

while true; do
  for active_file in "$TIMELAPSES_DIR"/*/.active; do
    [ -f "$active_file" ] || continue

    PRINTER_DIR=$(dirname "$active_file")
    PRINTER_ID=$(basename "$PRINTER_DIR")
    FRAMES_DIR="$PRINTER_DIR/frames"
    RTSP_URL=$(cat "$PRINTER_DIR/.rtsp_url" 2>/dev/null || echo "")
    PID_FILE="$PRINTER_DIR/.capture_pid"

    [ -z "$RTSP_URL" ] && continue

    # Skip if capture process is already running
    if [ -f "$PID_FILE" ] && kill -0 "$(cat "$PID_FILE")" 2>/dev/null; then
      continue
    fi

    mkdir -p "$FRAMES_DIR"
    FRAME_COUNT=$(ls "$FRAMES_DIR"/*.png 2>/dev/null | wc -l || echo "0")

    echo "Capturing frame $FRAME_COUNT for $PRINTER_ID"

    # Grab a single frame
    ffmpeg -rtsp_transport tcp -i "$RTSP_URL" \
      -vframes 1 -q:v 2 \
      "$FRAMES_DIR/frame_$(printf '%05d' $FRAME_COUNT).png" \
      < /dev/null > /dev/null 2>&1 &

    echo $! > "$PID_FILE"
  done

  # Check for completed timelapses (.stitch file = signal to create video)
  for stitch_file in "$TIMELAPSES_DIR"/*/.stitch; do
    [ -f "$stitch_file" ] || continue

    PRINTER_DIR=$(dirname "$stitch_file")
    PRINTER_ID=$(basename "$PRINTER_DIR")
    FRAMES_DIR="$PRINTER_DIR/frames"
    OUTPUT="$PRINTER_DIR/timelapse.mp4"

    FRAME_COUNT=$(ls "$FRAMES_DIR"/*.png 2>/dev/null | wc -l || echo "0")

    if [ "$FRAME_COUNT" -gt 1 ]; then
      echo "Stitching timelapse for $PRINTER_ID ($FRAME_COUNT frames)"
      ffmpeg -y -framerate 30 -pattern_type glob \
        -i "$FRAMES_DIR/frame_*.png" \
        -c:v libx264 -pix_fmt yuv420p \
        "$OUTPUT" < /dev/null > /dev/null 2>&1
      echo "Timelapse created: $OUTPUT"
    fi

    rm -f "$stitch_file"
  done

  sleep "$INTERVAL"
done
