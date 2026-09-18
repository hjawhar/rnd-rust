import { Component, input, OnInit, inject, signal, computed } from '@angular/core';
import { HttpClient } from '@angular/common/http';
import { TelemetrySnapshot } from '../../core/models/printer.model';

interface PrinterStats {
  total_prints: number;
  completed: number;
  failed: number;
  total_print_time_seconds: number;
  avg_print_time_seconds: number;
}

@Component({
  selector: 'app-stats-panel',
  template: `
    <div class="bg-surface rounded-lg p-3">
      <h3 class="text-xs font-medium text-text-muted uppercase tracking-wider mb-2">Statistics</h3>
      @if (stats(); as s) {
        <div class="grid grid-cols-2 gap-2 text-xs">
          <div>
            <div class="text-xl font-bold text-text">{{ s.total_prints }}</div>
            <div class="text-xs text-text-muted">Total Prints</div>
          </div>
          <div>
            <div class="text-xl font-bold text-success">{{ successRate() }}%</div>
            <div class="text-xs text-text-muted">Success Rate</div>
          </div>
          <div>
            <div class="text-base font-semibold text-text">{{ printTime() }}</div>
            <div class="text-xs text-text-muted">Print Time</div>
          </div>
          <div>
            <div class="text-base font-semibold text-text">{{ formatDuration(s.avg_print_time_seconds) }}</div>
            <div class="text-xs text-text-muted">Avg Print</div>
          </div>
        </div>
      } @else {
        <div class="text-sm text-text-muted">Loading...</div>
      }
    </div>
  `,
})
export class StatsPanelComponent implements OnInit {
  printerId = input.required<string>();
  snapshot = input<TelemetrySnapshot | null>(null);
  private http = inject(HttpClient);
  stats = signal<PrinterStats | null>(null);

  successRate = () => {
    const s = this.stats();
    if (!s || s.total_prints === 0) return 0;
    return Math.round((s.completed / s.total_prints) * 100);
  };

  /** Elapsed time of current print, derived from telemetry. */
  printTime = computed(() => {
    const s = this.snapshot();
    if (!s) return '--';
    const pct = s.print_progress_pct;
    const remaining = s.eta_minutes;
    if (pct > 0 && pct < 100 && remaining > 0) {
      const elapsed = Math.round(remaining * pct / (100 - pct));
      return this.formatDuration(elapsed * 60);
    }
    return '--';
  });

  ngOnInit() {
    this.http.get<PrinterStats>(`/api/printers/${this.printerId()}/stats`).subscribe({
      next: (s) => this.stats.set(s),
    });
  }

  formatDuration(seconds: number): string {
    if (seconds === 0) return '--';
    const h = Math.floor(seconds / 3600);
    const m = Math.floor((seconds % 3600) / 60);
    if (h > 24) {
      const d = Math.floor(h / 24);
      return `${d}d ${h % 24}h`;
    }
    return h > 0 ? `${h}h ${m}m` : `${m}m`;
  }
}
