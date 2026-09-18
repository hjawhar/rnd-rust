export interface PrinterConfig {
  id: string;
  name: string;
  ip: string;
  serial: string;
  access_code: string;
  model: string;
}

export interface Temperature {
  current: number;
  target: number;
}

export interface AmsStatus {
  units: AmsUnit[];
  tray_now: string;
  tray_tar: string;
  insert_flag: boolean;
  firmware_version: number;
}

export interface AmsUnit {
  id: string;
  humidity: string;
  temperature: number;
  trays: AmsTray[];
}

export interface AmsTray {
  id: string;
  filament_type: string;
  color: string;
  sub_brand: string;
  nozzle_temp_min: number;
  nozzle_temp_max: number;
  remain_pct: number;
  tag_uid: string;
  diameter: number;
  drying_temp: string;
  drying_time: string;
}

export interface LightStatus {
  node: string;
  mode: string;
}

export interface XcamSettings {
  spaghetti_detector: boolean;
  print_halt: boolean;
  first_layer_inspector: boolean;
  buildplate_marker_detector: boolean;
  printing_monitor: boolean;
}

export interface UpgradeState {
  ota_version: string;
  ams_version: string;
  status: string;
  progress: number;
  force_upgrade: boolean;
  new_version_state: number;
  err_code: number;
  module: string;
  message: string;
}

export interface HmsAlert {
  attr: number;
  code: number;
  description: string;
}

export interface NozzleInfo {
  type: string;
  diameter: number;
  wear: number;
}

export interface BuildPlate {
  base: number;
  material: number;
}

export interface NetworkInfo {
  wifi_signal: string;
  ip_address: string;
}

export interface PrinterIdentity {
  printer_id: string;
  name: string;
  firmware_version: string;
  model: string;
  serial_number: string;
}

export interface TelemetrySnapshot {
  printer: PrinterIdentity | null;
  timestamp_ms: number;
  state: number;
  gcode_state: string;
  nozzle_temp: Temperature | null;
  bed_temp: Temperature | null;
  chamber_temp: Temperature | null;
  print_progress_pct: number;
  current_file: string;
  eta_minutes: number;
  layer_num: number;
  total_layer_num: number;
  subtask_name: string;
  part_fan_pct: number;
  aux_fan_pct: number;
  chamber_fan_pct: number;
  heatbreak_fan_pct: number;
  fan_gear: number;
  speed_profile: number;
  speed_magnitude_pct: number;
  wifi_signal: string;
  ams: AmsStatus | null;
  lights: LightStatus[];
  rtsp_url: string;
  nozzle_type: string;
  nozzle_diameter: number;
  print_error: number;
  error_code: string;
  xcam: XcamSettings | null;
  sdcard_present: boolean;
  upgrade_state: UpgradeState | null;
  hms_alerts: HmsAlert[];
  nozzle_info: NozzleInfo | null;
  build_plate: BuildPlate | null;
  network: NetworkInfo | null;
}

export interface PrinterWithStatus {
  id: string;
  name: string;
  ip: string;
  serial: string;
  access_code: string;
  model: string;
  online: boolean;
  status: TelemetrySnapshot | null;
}

export enum PrinterState {
  Unspecified = 0,
  Idle = 1,
  Printing = 2,
  Paused = 3,
  Error = 4,
  Preparing = 5,
  Finishing = 6,
  Finished = 7,
  Failed = 8,
}

export enum SpeedProfile {
  Unspecified = 0,
  Silent = 1,
  Standard = 2,
  Sport = 3,
  Ludicrous = 4,
}

export function printerStateLabel(state: number): string {
  const labels: Record<number, string> = {
    0: 'Unknown',
    1: 'Idle',
    2: 'Printing',
    3: 'Paused',
    4: 'Error',
    5: 'Preparing',
    6: 'Finishing',
    7: 'Finished',
    8: 'Failed',
  };
  return labels[state] ?? 'Unknown';
}

export function speedProfileLabel(speed: number): string {
  const labels: Record<number, string> = {
    0: 'Unknown',
    1: 'Silent',
    2: 'Standard',
    3: 'Sport',
    4: 'Ludicrous',
  };
  return labels[speed] ?? 'Unknown';
}
