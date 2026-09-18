import { Component, inject, signal, OnInit } from '@angular/core';
import { RouterLink } from '@angular/router';
import { PrinterService } from '../../core/services/printer.service';
import { PrinterWithStatus } from '../../core/models/printer.model';
import { StatusBadgeComponent } from '../../shared/components/status-badge.component';
import { forkJoin } from 'rxjs';
import { FilamentPanelComponent } from './filament-panel.component';

interface FleetStats {
  totalPrinters: number;
  online: number;
  printing: number;
  idle: number;
  errors: number;
  totalPrints: number;
  totalPrintTimeMinutes: number;
}

@Component({
  selector: 'app-fleet',
  imports: [RouterLink, StatusBadgeComponent, FilamentPanelComponent],
  template: `
    <div class="p-6">
      <div class="flex items-center justify-between mb-6">
        <h1 class="text-2xl font-bold text-text">Fleet Overview</h1>
        <a routerLink="/" class="text-sm text-primary hover:text-primary-light">← Dashboard</a>
      </div>

      @if (stats(); as s) {
        <div class="grid grid-cols-2 md:grid-cols-4 lg:grid-cols-7 gap-4 mb-8">
          <div class="bg-surface-light rounded-lg border border-border p-4 text-center">
            <div class="text-3xl font-bold text-text">{{ s.totalPrinters }}</div>
            <div class="text-xs text-text-muted">Printers</div>
          </div>
          <div class="bg-surface-light rounded-lg border border-border p-4 text-center">
            <div class="text-3xl font-bold text-success">{{ s.online }}</div>
            <div class="text-xs text-text-muted">Online</div>
          </div>
          <div class="bg-surface-light rounded-lg border border-border p-4 text-center">
            <div class="text-3xl font-bold text-primary">{{ s.printing }}</div>
            <div class="text-xs text-text-muted">Printing</div>
          </div>
          <div class="bg-surface-light rounded-lg border border-border p-4 text-center">
            <div class="text-3xl font-bold text-text-muted">{{ s.idle }}</div>
            <div class="text-xs text-text-muted">Idle</div>
          </div>
          <div class="bg-surface-light rounded-lg border border-border p-4 text-center">
            <div class="text-3xl font-bold text-error">{{ s.errors }}</div>
            <div class="text-xs text-text-muted">Errors</div>
          </div>
          <div class="bg-surface-light rounded-lg border border-border p-4 text-center">
            <div class="text-3xl font-bold text-text">{{ s.totalPrints }}</div>
            <div class="text-xs text-text-muted">Total Prints</div>
          </div>
          <div class="bg-surface-light rounded-lg border border-border p-4 text-center">
            <div class="text-3xl font-bold text-text">{{ formatEta(s.totalPrintTimeMinutes) }}</div>
            <div class="text-xs text-text-muted">Print Time</div>
          </div>
        </div>
      }

      @if (printers().length > 0) {
        <div class="bg-surface-light rounded-lg border border-border overflow-x-auto">
          <table class="w-full text-sm">
            <thead>
              <tr class="border-b border-border">
                <th class="text-left p-3 text-text-muted font-medium">Printer</th>
                <th class="text-left p-3 text-text-muted font-medium">Status</th>
                <th class="text-right p-3 text-text-muted font-medium">Nozzle</th>
                <th class="text-right p-3 text-text-muted font-medium">Bed</th>
                <th class="text-right p-3 text-text-muted font-medium">Progress</th>
                <th class="text-left p-3 text-text-muted font-medium">File</th>
                <th class="text-right p-3 text-text-muted font-medium">ETA</th>
              </tr>
            </thead>
            <tbody>
              @for (p of printers(); track p.id) {
                <tr class="border-b border-border/50 hover:bg-surface-lighter cursor-pointer" [routerLink]="['/printers', p.id]">
                  <td class="p-3">
                    <div class="text-text font-medium">{{ p.name }}</div>
                    <div class="text-xs text-text-muted">{{ p.model }}</div>
                  </td>
                  <td class="p-3"><app-status-badge [state]="p.status?.state ?? 0" [online]="p.online" /></td>
                  <td class="p-3 text-right text-text">{{ p.status?.nozzle_temp?.current?.toFixed(0) ?? '--' }}°</td>
                  <td class="p-3 text-right text-text">{{ p.status?.bed_temp?.current?.toFixed(0) ?? '--' }}°</td>
                  <td class="p-3 text-right text-text">{{ p.status?.state === 2 ? p.status?.print_progress_pct + '%' : '--' }}</td>
                  <td class="p-3 text-text truncate max-w-48">{{ p.status?.subtask_name || '--' }}</td>
                  <td class="p-3 text-right text-text-muted">{{ formatEta(p.status?.eta_minutes ?? 0) }}</td>
                </tr>
              }
            </tbody>
          </table>
        </div>
      }

      <div class="mt-6">
        <app-filament-panel />
      </div>
    </div>
  `,
})
export class FleetPage implements OnInit {
  private printerService = inject(PrinterService);
  printers = signal<PrinterWithStatus[]>([]);
  stats = signal<FleetStats | null>(null);

  ngOnInit() {
    this.printerService.listPrinters().subscribe({
      next: (printers) => {
        this.printers.set(printers);
        const s: FleetStats = {
          totalPrinters: printers.length,
          online: printers.filter(p => p.online).length,
          printing: printers.filter(p => p.status?.state === 2).length,
          idle: printers.filter(p => p.status?.state === 1).length,
          errors: printers.filter(p => p.status?.state === 4).length,
          totalPrints: 0,
          totalPrintTimeMinutes: this.computeFleetPrintTimeMinutes(printers),
        };
        if (printers.length > 0) {
          forkJoin(printers.map(p => this.printerService.getStats(p.id))).subscribe({
            next: (allStats) => {
              let prints = 0;
              for (const st of allStats) {
                prints += st.total_prints ?? 0;
              }
              s.totalPrints = prints;
              this.stats.set(s);
            },
            error: () => this.stats.set(s),
          });
        } else {
          this.stats.set(s);
        }
      },
    });
  }

  /** Sum elapsed print time across all actively printing printers, derived from telemetry. */
  private computeFleetPrintTimeMinutes(printers: PrinterWithStatus[]): number {
    let totalMinutes = 0;
    for (const p of printers) {
      const pct = p.status?.print_progress_pct ?? 0;
      const remaining = p.status?.eta_minutes ?? 0;
      if (pct > 0 && pct < 100 && remaining > 0) {
        totalMinutes += Math.round(remaining * pct / (100 - pct));
      }
    }
    return totalMinutes;
  }

  formatEta(minutes: number): string {
    if (!minutes) return '--';
    if (minutes < 60) return `${minutes}m`;
    const h = Math.floor(minutes / 60);
    const m = minutes % 60;
    return m > 0 ? `${h}h ${m}m` : `${h}h`;
  }
}
