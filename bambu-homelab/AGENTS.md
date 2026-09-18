# Bambu Homelab -- Developer Reference

See [README.md](README.md) for installation, setup, and usage instructions.

## Architecture

```
Printers (MQTT/JSON on LAN:8883)
    |
Bridge (host network, multi-printer connection manager)
    | protobuf over NATS JetStream (TLS + NKey auth)
Gateway (JetStream durable consumer)
    |
API (axum -- REST + WebSocket + JWT auth + Diesel/Postgres)
    | JSON over HTTP/WS
Dashboard (Angular 19 + Tailwind v4)
```

All Rust services run in Docker. Angular runs locally with `ng serve`.

## Components

### Bridge (`crates/bridge/`)
Multi-printer connection manager. Connects to Postgres on startup, loads all
registered printers, spawns an MQTT connection per printer. Listens on NATS
`bridge.events` for dynamic add/remove from the API. Each printer connection:
- Subscribes to `device/{serial}/report` on the printer's MQTT broker
- Translates Bambu JSON to protobuf `TelemetrySnapshot`
- Publishes to `printers.{id}.telemetry` on NATS
- Listens on `printers.{id}.cmd` for commands (pause/resume/stop/speed/lights/home/upgrade/gcode/print_start)
- Translates commands to Bambu MQTT JSON and publishes to `device/{serial}/request`
- Publishes heartbeat every 30s to `printers.{id}.heartbeat`
- Auto-reconnects on MQTT disconnect

### Gateway (`crates/gateway/`)
JetStream durable consumer for printer telemetry. Creates a `PRINTER_TELEMETRY`
stream on startup. Decodes protobuf and logs structured telemetry.

### API (`crates/api/`)
HTTP + WebSocket server. Axum 0.8 with JWT authentication.
- **Auth**: `POST /api/auth/login`, `POST /api/auth/password`
- **Printers**: `GET/POST/DELETE /api/printers`, `GET /api/printers/{id}`
- **Commands**: `POST /api/printers/{id}/command`
- **Files**: `GET /api/printers/{id}/files` (FTPS), `POST /api/printers/{id}/upload` (multipart)
- **Print**: `POST /api/printers/{id}/print`
- **History**: `GET /api/printers/{id}/history`
- **Stats**: `GET /api/printers/{id}/stats`
- **Queue**: `GET/POST /api/printers/{id}/queue`, `DELETE /api/printers/{id}/queue/{qid}`
- **Users**: `GET/POST /api/users`, `DELETE /api/users/{id}` (admin only)
- **Assignments**: `GET/POST /api/printers/{id}/assignments`, `DELETE /api/printers/{id}/assignments/{user_id}` (admin only)
- **WebSocket**: `WS /api/ws` -- JWT auth via first message, subscribe/unsubscribe per printer, real-time telemetry + assignment events
- **Health**: `GET /api/health`

Persistence: Diesel + diesel-async + bb8 pool (Postgres).
Migrations run automatically on startup (inline SQL, no diesel_cli needed).
Admin user auto-created on first run (password printed to stdout).

### Shared (`crates/shared/`)
Protobuf definitions (prost), config types, error types. Shared across all crates.

### Dashboard (`dashboard/`)
Angular 19 standalone components with Tailwind CSS v4. Dark theme by default.
- **Login page**: JWT auth, token stored in localStorage
- **Dashboard page**: printer grid with live status cards, online/offline dots, AMS badges
- **Printer detail page**: 12 panels -- temperatures (with sparklines), print progress,
  AMS (humidity + trays), controls, versions, HMS alerts, info, camera, history, stats, print start
- **Add printer page**: form with validation, registers via API
- **Keyboard shortcuts**: space=pause, s=stop, 1-4=speed, l=light
- **Browser notifications**: print complete, failed, paused
- **Connection indicator**: WebSocket status in nav bar

## Tech stack (decided -- do not suggest alternatives)
- Language: Rust (all backend services)
- Transport: NATS JetStream (not MQTT for inter-service)
- Auth: JWT (HS256) for users, NKeys for NATS
- Wire protocol: protobuf (prost) for bridge-to-NATS messages
- Database: Postgres via Diesel 2.2 + diesel-async + bb8
- Password hashing: argon2
- HTTP: axum 0.8 (ws, multipart features)
- FTPS: suppaftp (implicit TLS to printer port 990)
- MQTT: rumqttc (TLS to printer port 8883)
- Config: config-rs (env vars with `__` separator)
- Logging: tracing + tracing-subscriber
- Frontend: Angular 19 (standalone, signals), Tailwind CSS v4
- Runtime: Docker Compose (all services)

## Printer connection details (Bambu X1C)
- MQTT: port 8883, TLS (self-signed BBL CA, skip verification), username `bblp`, password = LAN access code
- FTPS: port 990 (implicit TLS, self-signed, skip verification), same credentials
- RTSP: `rtsps://{ip}:322/streaming/live/1` for camera
- Subscribe: `device/{serial}/report` for status (JSON)
- Publish: `device/{serial}/request` for commands (JSON)

## NATS subject hierarchy
```
printers.{id}.telemetry   -- bridge publishes protobuf status
printers.{id}.heartbeat   -- bridge publishes "alive" every 30s
printers.{id}.cmd         -- API publishes command JSON
printers.{id}.events      -- print start/finish/error (future)
bridge.events              -- API publishes printer_added/printer_removed
```

## Database tables
- `users`: id (uuid), username, password_hash (argon2), role (admin/user), created_at
- `printers`: id (serial), name, ip, serial, access_code, model, owner_id (FK users), created_at
- `print_jobs`: id (uuid), printer_id, file_name, started_at, finished_at, status, total_layers, duration_seconds
- `print_queue`: id (uuid), printer_id, file_name, plate_number, status, position, created_at
- `filament_usage`: id (uuid), print_job_id, printer_id, filament_type, color, weight_grams, created_at
- `printer_assignments`: id (uuid), user_id (FK users), printer_id (FK printers), created_at -- UNIQUE(user_id, printer_id)

Migrations run inline on API startup. No diesel_cli needed.

## Design principles
- Printers managed entirely through the API/database -- no config files with printer info.
- Bridge connects outbound to printers' LAN MQTT -- no inbound ports needed on printers.
- Multi-printer: bridge spawns independent connection per printer, supports 50+.
- Commands are typed and translated at the bridge layer -- API sends generic commands.
- Admin auto-created on first startup -- no seed scripts.
- All infrastructure in Docker -- nothing installed on the host except Docker and Node.js.
- No native mobile app -- browser-based, fully responsive via Tailwind.
- No custom firmware -- the X1C's LAN MQTT gives us everything.
- Online status is time-based (60s heartbeat threshold), not boolean.
- Role-based access: admin has full control, users see only assigned printers (read-only).
- WebSocket requires JWT auth as first message. Unauthenticated connections closed after 5s.
- Root admin (auto-bootstrapped) cannot be deleted. New users always created with role 'user'.
- Print time derived from live printer telemetry (remaining time + progress), not database records.
