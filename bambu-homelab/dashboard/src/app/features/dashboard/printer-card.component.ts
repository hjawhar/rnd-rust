import { Component, input } from '@angular/core';
import { RouterLink } from '@angular/router';
import { PrinterWithStatus } from '../../core/models/printer.model';
import { StatusBadgeComponent } from '../../shared/components/status-badge.component';

@Component({
  selector: 'app-printer-card',
  imports: [RouterLink, StatusBadgeComponent],
  template: `
    <a [routerLink]="['/printers', printer().id]"
       class="block bg-surface-light rounded-lg border border-border p-4 hover:border-primary/50 transition-colors">
      <div class="flex items-center justify-between mb-3">
        <div class="flex items-center gap-2">
          <span [class]="printer().online ? 'w-2 h-2 rounded-full bg-success' : 'w-2 h-2 rounded-full bg-error'"
                [title]="printer().online ? 'Online' : 'Offline'"></span>
          <h3 class="font-semibold text-text truncate">{{ printer().name }}</h3>
        </div>
        <app-status-badge [state]="printer().status?.state ?? 0" [online]="printer().online" />
      </div>

      <div class="text-sm text-text-muted mb-2">{{ printer().model }} &bull; {{ printer().serial }}</div>

      @if (printer().status; as s) {
        <div class="grid grid-cols-2 gap-2 text-sm mb-3">
          <div>
            <span class="text-text-muted">Nozzle</span>
            <span class="ml-1 text-text">{{ s.nozzle_temp?.current?.toFixed(0) ?? '--' }}&deg;C</span>
          </div>
          <div>
            <span class="text-text-muted">Bed</span>
            <span class="ml-1 text-text">{{ s.bed_temp?.current?.toFixed(0) ?? '--' }}&deg;C</span>
          </div>
        </div>

        @if (s.state === 2) {
          <div class="mt-2">
            <div class="flex justify-between text-xs text-text-muted mb-1">
              <span>{{ s.subtask_name || 'Printing' }}</span>
              <span>{{ s.print_progress_pct }}%</span>
            </div>
            <div class="w-full bg-surface rounded-full h-1.5">
              <div class="bg-primary rounded-full h-1.5 transition-all" [style.width.%]="s.print_progress_pct"></div>
            </div>
            <div class="flex justify-between text-xs text-text-muted mt-1">
              @if (elapsedMinutes(s) > 0) {
                <span>Elapsed: {{ formatEta(elapsedMinutes(s)) }}</span>
              }
              @if (s.eta_minutes > 0) {
                <span>ETA: {{ formatEta(s.eta_minutes) }}</span>
              }
            </div>
          </div>
        }

        @if (s.ams) {
          <div class="flex items-center gap-1 mt-2">
            <span class="text-xs text-text-muted">AMS</span>
            @for (unit of s.ams.units; track unit.id) {
              @for (tray of unit.trays; track tray.id) {
                <div class="w-3 h-3 rounded-full border border-border"
                     [style.background-color]="'#' + tray.color.slice(0, 6)"
                     [title]="tray.filament_type"></div>
              }
            }
          </div>
        }
      }
    </a>
  `,
})
export class PrinterCardComponent {
  printer = input.required<PrinterWithStatus>();

  formatEta(minutes: number): string {
    if (minutes < 60) return `${minutes}m`;
    const h = Math.floor(minutes / 60);
    const m = minutes % 60;
    return m > 0 ? `${h}h ${m}m` : `${h}h`;
  }

  elapsedMinutes(s: { print_progress_pct: number; eta_minutes: number }): number {
    if (s.print_progress_pct <= 0 || s.print_progress_pct >= 100 || s.eta_minutes <= 0) return 0;
    return Math.round(s.eta_minutes * s.print_progress_pct / (100 - s.print_progress_pct));
  }
}
