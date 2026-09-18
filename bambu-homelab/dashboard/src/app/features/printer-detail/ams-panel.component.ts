import { Component, input } from '@angular/core';
import { AmsStatus } from '../../core/models/printer.model';

@Component({
  selector: 'app-ams-panel',
  template: `
    <div class="bg-surface-light rounded-lg border border-border p-3">
      <div class="flex items-center justify-between mb-2">
        <span class="text-xs font-medium text-text-muted uppercase tracking-wider">AMS</span>
        @if (ams()) {
          <span class="text-[10px] text-text-muted">v{{ ams()!.firmware_version }}</span>
        }
      </div>
      @if (ams(); as a) {
        @for (unit of a.units; track unit.id) {
          <div class="mb-2 last:mb-0">
            <div class="flex items-center gap-2 text-[10px] text-text-muted mb-1.5">
              <span>Unit {{ unit.id }}</span>
              <span [class]="humidityClass(unit.humidity)">
                Humidity: {{ unit.humidity }}%
              </span>
            </div>
            <div class="grid grid-cols-4 gap-1.5">
              @for (tray of unit.trays; track tray.id) {
                <div class="bg-surface rounded p-1.5 text-center">
                  <div class="w-4 h-4 rounded-full mx-auto mb-0.5 border border-border/50"
                       [style.background-color]="'#' + tray.color.slice(0, 6)"></div>
                  <div class="text-[10px] text-text truncate leading-tight">{{ tray.filament_type || '--' }}</div>
                  @if (tray.drying_temp && tray.drying_temp !== '0') {
                    <div class="text-[10px] text-warning leading-tight">Drying</div>
                  }
                </div>
              }
            </div>
          </div>
        }
      } @else {
        <div class="text-xs text-text-muted">No AMS connected</div>
      }
    </div>
  `,
})
export class AmsPanelComponent {
  ams = input<AmsStatus | null>(null);

  humidityClass(humidity: string): string {
    const val = parseInt(humidity, 10);
    if (isNaN(val)) return 'px-1 py-0.5 rounded text-[10px] font-medium text-text-muted';
    if (val <= 3) return 'px-1 py-0.5 rounded text-[10px] font-medium text-success';
    if (val <= 5) return 'px-1 py-0.5 rounded text-[10px] font-medium text-warning';
    return 'px-1 py-0.5 rounded text-[10px] font-medium text-error';
  }
}
