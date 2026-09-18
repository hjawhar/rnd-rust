#!/bin/bash
# Video relay: converts RTSP streams from Bambu printers to HLS.
# Streams are served from /streams/{printer_id}/stream.m3u8
#
# This script is started by docker-compose and watches for stream
# requests via a simple control mechanism.
#
# For simplicity, this version starts a single ffmpeg process per
# printer. In production, use a proper media server like MediaMTX.

set -e

STREAMS_DIR=/streams
mkdir -p "$STREAMS_DIR"

echo "Video relay ready. Streams directory: $STREAMS_DIR"
echo "To start a stream, create a file: $STREAMS_DIR/{printer_id}.conf"
echo "with content: RTSP_URL=rtsps://ip:322/streaming/live/1"

# Watch for stream config files and start ffmpeg
while true; do
  for conf in "$STREAMS_DIR"/*.conf; do
    [ -f "$conf" ] || continue

    PRINTER_ID=$(basename "$conf" .conf)
    STREAM_DIR="$STREAMS_DIR/$PRINTER_ID"
    PID_FILE="$STREAM_DIR/.pid"

    # Skip if already running
    if [ -f "$PID_FILE" ] && kill -0 "$(cat "$PID_FILE")" 2>/dev/null; then
      continue
    fi

    # Read config
    source "$conf"

    if [ -z "$RTSP_URL" ]; then
      continue
    fi

    mkdir -p "$STREAM_DIR"

    echo "Starting HLS stream for $PRINTER_ID from $RTSP_URL"

    ffmpeg -rtsp_transport tcp \
      -i "$RTSP_URL" \
      -c:v copy \
      -f hls \
      -hls_time 2 \
      -hls_list_size 3 \
      -hls_flags delete_segments+append_list \
      -hls_segment_filename "$STREAM_DIR/segment_%03d.ts" \
      "$STREAM_DIR/stream.m3u8" \
      < /dev/null > "$STREAM_DIR/ffmpeg.log" 2>&1 &

    echo $! > "$PID_FILE"
  done

  sleep 5
done
