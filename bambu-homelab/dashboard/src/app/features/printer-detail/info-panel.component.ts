import { Component, input, computed } from '@angular/core';
import { TelemetrySnapshot } from '../../core/models/printer.model';

@Component({
  selector: 'app-info-panel',
  template: `
    <div class="bg-surface rounded-lg p-3">
      <h3 class="text-xs font-medium text-text-muted uppercase tracking-wider mb-2">Info</h3>
      <div class="space-y-1.5 text-xs">
        @for (item of infoItems(); track item.label) {
          <div class="flex justify-between">
            <span class="text-text-muted">{{ item.label }}</span>
            <span class="text-text">{{ item.value }}</span>
          </div>
        }
      </div>
    </div>
  `,
})
export class InfoPanelComponent {
  snapshot = input.required<TelemetrySnapshot>();

  infoItems = computed(() => {
    const s = this.snapshot();
    const items = [
      { label: 'Serial', value: s.printer?.serial_number ?? '--' },
      { label: 'Nozzle', value: `${s.nozzle_info?.type ?? s.nozzle_type} ${s.nozzle_info?.diameter?.toFixed(1) ?? s.nozzle_diameter?.toFixed(1) ?? '0.4'}mm` },
    ];
    if (s.nozzle_info?.wear) {
      items.push({ label: 'Nozzle Wear', value: `${s.nozzle_info.wear}` });
    }
    if (s.build_plate) {
      const plates: Record<number, string> = { 1: 'Cool Plate', 2: 'Engineering', 3: 'High Temp' };
      items.push({ label: 'Build Plate', value: plates[s.build_plate.material] ?? `Type ${s.build_plate.material}` });
    }
    items.push({ label: 'WiFi', value: s.network?.wifi_signal ?? s.wifi_signal ?? '--' });
    if (s.network?.ip_address) {
      items.push({ label: 'IP', value: s.network.ip_address });
    }
    items.push({ label: 'SD Card', value: s.sdcard_present ? 'Present' : 'Not found' });
    return items;
  });
}
