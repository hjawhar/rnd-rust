import { Component, input, computed } from '@angular/core';
import { TelemetrySnapshot, speedProfileLabel } from '../../core/models/printer.model';

@Component({
  selector: 'app-print-progress',
  template: `
    <div class="bg-surface-light rounded-lg border border-border p-3">
      @if (isPrinting()) {
        <div class="text-xs text-text font-medium truncate">{{ snapshot().subtask_name || snapshot().current_file }}</div>
        <div class="flex justify-between text-xs text-text-muted">
          <span>Layer {{ snapshot().layer_num }} / {{ snapshot().total_layer_num }}</span>
          <span class="font-semibold text-text">{{ snapshot().print_progress_pct }}%</span>
        </div>
        <div class="w-full bg-surface rounded-full h-1.5 my-1.5">
          <div class="bg-primary rounded-full h-1.5 transition-all" [style.width.%]="snapshot().print_progress_pct"></div>
        </div>
        <div class="flex justify-between text-[10px] text-text-muted">
          @if (elapsedMinutes() > 0) {
            <span>Elapsed: {{ formatEta(elapsedMinutes()) }}</span>
          }
          @if (snapshot().eta_minutes > 0) {
            <span>ETA: {{ formatEta(snapshot().eta_minutes) }}</span>
          }
          <span>Speed: {{ speedLabel() }}</span>
        </div>
      } @else {
        <div class="text-xs text-text-muted">{{ stateLabel() }}</div>
      }
    </div>
  `,
})
export class PrintProgressComponent {
  snapshot = input.required<TelemetrySnapshot>();

  isPrinting = computed(() => {
    const s = this.snapshot().state;
    return s === 2 || s === 5; // Printing or Preparing
  });

  speedLabel = computed(() => speedProfileLabel(this.snapshot().speed_profile));

  elapsedMinutes = computed(() => {
    const pct = this.snapshot().print_progress_pct;
    const remaining = this.snapshot().eta_minutes;
    if (pct <= 0 || pct >= 100 || remaining <= 0) return 0;
    // elapsed = remaining * pct / (100 - pct)
    return Math.round(remaining * pct / (100 - pct));
  });

  stateLabel = computed(() => {
    const s = this.snapshot().gcode_state;
    return s || 'Idle';
  });

  formatEta(minutes: number): string {
    if (minutes < 60) return `${minutes}m`;
    const h = Math.floor(minutes / 60);
    const m = minutes % 60;
    return m > 0 ? `${h}h ${m}m` : `${h}h`;
  }
}
