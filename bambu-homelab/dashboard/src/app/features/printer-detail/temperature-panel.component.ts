import { Component, input, computed, inject } from '@angular/core';
import { TelemetrySnapshot } from '../../core/models/printer.model';
import { TelemetryStore } from '../../core/services/telemetry.store';
import { SparklineComponent } from '../../shared/components/sparkline.component';

@Component({
  selector: 'app-temperature-panel',
  imports: [SparklineComponent],
  template: `
    <div class="bg-surface-light rounded-lg border border-border p-3">
      <div class="text-xs font-medium text-text-muted uppercase tracking-wider mb-2">Temperatures</div>
      <div class="grid grid-cols-3 gap-3">
        @for (temp of temps(); track temp.label) {
          <div>
            <div class="flex items-baseline gap-1.5">
              <span class="text-lg font-semibold text-text">{{ temp.current }}°</span>
              @if (temp.target > 0) {
                <span class="text-[10px] text-text-muted">→ {{ temp.target }}°</span>
              }
            </div>
            <div class="text-[10px] text-text-muted mt-0.5">{{ temp.label }}</div>
            @if (temp.history.length > 1) {
              <app-sparkline [data]="temp.history" [color]="temp.color" [width]="80" [height]="20" />
            }
          </div>
        }
      </div>
    </div>
  `,
})
export class TemperaturePanelComponent {
  snapshot = input.required<TelemetrySnapshot>();
  printerId = input.required<string>();

  private store = inject(TelemetryStore);

  temps = computed(() => {
    const s = this.snapshot();
    const history = this.store.getHistory(this.printerId());
    return [
      {
        label: 'Nozzle',
        current: s.nozzle_temp?.current?.toFixed(0) ?? '--',
        target: s.nozzle_temp?.target ?? 0,
        history: history?.nozzle ?? [],
        color: '#ef4444',
      },
      {
        label: 'Bed',
        current: s.bed_temp?.current?.toFixed(0) ?? '--',
        target: s.bed_temp?.target ?? 0,
        history: history?.bed ?? [],
        color: '#f59e0b',
      },
      {
        label: 'Chamber',
        current: s.chamber_temp?.current?.toFixed(0) ?? '--',
        target: s.chamber_temp?.target ?? 0,
        history: history?.chamber ?? [],
        color: '#3b82f6',
      },
    ];
  });
}
