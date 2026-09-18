import { Component, input, output, computed } from '@angular/core';
import { TelemetrySnapshot } from '../../core/models/printer.model';

@Component({
  selector: 'app-version-panel',
  template: `
    <div class="bg-surface rounded-lg p-3">
      <h3 class="text-xs font-medium text-text-muted uppercase tracking-wider mb-2">Firmware</h3>
      <div class="space-y-1.5 text-xs">
        <div class="flex justify-between">
          <span class="text-text-muted">Printer FW</span>
          <span class="text-text font-mono">{{ printerVersion() }}</span>
        </div>
        <div class="flex justify-between">
          <span class="text-text-muted">AMS FW</span>
          <span class="text-text font-mono">{{ amsVersion() }}</span>
        </div>

        @if (isUpdating()) {
          <div class="mt-2 p-2 bg-primary/10 rounded-lg">
            <div class="flex justify-between text-xs mb-1">
              <span class="text-primary">{{ updateStatus() }}</span>
              <span class="text-text">{{ updateProgress() }}%</span>
            </div>
            <div class="w-full bg-surface rounded-full h-2">
              <div class="bg-primary rounded-full h-2 transition-all" [style.width.%]="updateProgress()"></div>
            </div>
            @if (updateModule()) {
              <div class="text-xs text-text-muted mt-1">Module: {{ updateModule() }}</div>
            }
          </div>
        } @else if (hasUpdate() && !readOnly()) {
          <div class="mt-2 p-2 bg-primary/10 rounded-lg">
            <div class="text-xs text-primary mb-1">Update available</div>
            @if (newVersion()) {
              <div class="text-xs text-text-muted mb-2">New version: {{ newVersion() }}</div>
            }
            <button (click)="confirmUpgrade()"
                    class="w-full px-3 py-1.5 bg-primary text-white rounded text-xs font-medium hover:bg-primary-dark">
              Update Firmware
            </button>
          </div>
        } @else {
          <div class="mt-1 text-xs text-success">Up to date</div>
        }
      </div>
    </div>
  `,
})
export class VersionPanelComponent {
  snapshot = input.required<TelemetrySnapshot>();
  onUpgrade = output<void>();
  readOnly = input(false);

  /** Format printer firmware version from integer (e.g. 20000 -> "v2.00.00") */
  printerVersion = computed(() => {
    // The `ver` field is not in upgrade_state — it's a top-level telemetry field
    // stored as firmware_version on PrinterIdentity, or we can compute from upgrade_state
    const u = this.snapshot().upgrade_state;
    if (u?.ota_version) return u.ota_version;

    // Fallback: format the identity firmware_version or show the raw ver number
    const fw = this.snapshot().printer?.firmware_version;
    if (fw && fw !== '0' && fw !== '') return fw;

    return 'Unknown';
  });

  /** Format AMS firmware version from integer (e.g. 723 -> "v7.23") */
  amsVersion = computed(() => {
    const u = this.snapshot().upgrade_state;
    if (u?.ams_version) return u.ams_version;

    const ams = this.snapshot().ams;
    if (ams?.firmware_version) {
      const v = ams.firmware_version;
      if (v > 0) {
        const major = Math.floor(v / 100);
        const minor = v % 100;
        return `v${major}.${minor.toString().padStart(2, '0')}`;
      }
    }
    return 'Unknown';
  });

  hasUpdate = computed(() => {
    const u = this.snapshot().upgrade_state;
    return u && u.new_version_state === 2;
  });

  isUpdating = computed(() => {
    const u = this.snapshot().upgrade_state;
    return u && u.status !== 'IDLE' && u.status !== '' && u.progress > 0;
  });

  updateProgress = computed(() => {
    return this.snapshot().upgrade_state?.progress ?? 0;
  });

  updateStatus = computed(() => {
    const status = this.snapshot().upgrade_state?.status ?? '';
    const labels: Record<string, string> = {
      'DOWNLOADING': 'Downloading...',
      'FLASHING': 'Installing...',
      'IDLE': 'Idle',
      'UPGRADING': 'Upgrading...',
    };
    return labels[status] ?? status;
  });

  updateModule = computed(() => {
    return this.snapshot().upgrade_state?.module ?? '';
  });

  newVersion = computed(() => {
    const u = this.snapshot().upgrade_state;
    return u?.ota_version || '';
  });

  confirmUpgrade() {
    if (confirm('Update firmware? The printer will restart during the update.')) {
      this.onUpgrade.emit();
    }
  }
}
