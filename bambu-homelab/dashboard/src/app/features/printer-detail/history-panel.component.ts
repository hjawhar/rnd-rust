import { Component, input, OnInit, inject, signal } from '@angular/core';
import { HttpClient } from '@angular/common/http';
import { DatePipe } from '@angular/common';

interface PrintJob {
  id: string;
  printer_id: string;
  file_name: string;
  started_at: string;
  finished_at: string | null;
  status: string;
  total_layers: number;
  duration_seconds: number | null;
}

@Component({
  selector: 'app-history-panel',
  imports: [DatePipe],
  template: `
    <div class="bg-surface rounded-lg p-3">
      <h3 class="text-xs font-medium text-text-muted uppercase tracking-wider mb-2">Print History</h3>
      @if (jobs().length > 0) {
        <div class="space-y-1.5 max-h-48 overflow-y-auto">
          @for (job of jobs(); track job.id) {
            <div class="p-2 rounded bg-surface text-xs flex justify-between items-center">
              <div>
                <div class="text-text font-medium truncate">{{ job.file_name || 'Unknown' }}</div>
                <div class="text-xs text-text-muted">{{ job.started_at | date:'short' }}</div>
              </div>
              <div class="text-right">
                <span [class]="statusClass(job.status)">{{ job.status }}</span>
                @if (job.duration_seconds) {
                  <div class="text-xs text-text-muted">{{ formatDuration(job.duration_seconds) }}</div>
                }
              </div>
            </div>
          }
        </div>
      } @else {
        <div class="text-sm text-text-muted">No print history yet</div>
      }
    </div>
  `,
})
export class HistoryPanelComponent implements OnInit {
  printerId = input.required<string>();
  private http = inject(HttpClient);
  jobs = signal<PrintJob[]>([]);

  ngOnInit() {
    this.http.get<PrintJob[]>(`/api/printers/${this.printerId()}/history`).subscribe({
      next: (jobs) => this.jobs.set(jobs),
    });
  }

  statusClass(status: string): string {
    const base = 'px-1.5 py-0.5 rounded text-xs font-medium';
    if (status === 'completed') return `${base} bg-success/20 text-success`;
    if (status === 'failed') return `${base} bg-error/20 text-error`;
    return `${base} bg-primary/20 text-primary`;
  }

  formatDuration(seconds: number): string {
    const h = Math.floor(seconds / 3600);
    const m = Math.floor((seconds % 3600) / 60);
    return h > 0 ? `${h}h ${m}m` : `${m}m`;
  }
}
