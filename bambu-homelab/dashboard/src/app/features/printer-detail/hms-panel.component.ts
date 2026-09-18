import { Component, input, signal, HostListener, ElementRef, inject } from '@angular/core';
import { HmsAlert } from '../../core/models/printer.model';

@Component({
  selector: 'app-hms-panel',
  template: `
    <div class="relative">
      <button (click)="toggle($event)" class="relative p-1.5 rounded-lg hover:bg-surface-lighter transition-colors">
        <svg class="w-4 h-4" [class]="alerts().length > 0 ? 'text-warning' : 'text-text-muted'" fill="none" stroke="currentColor" viewBox="0 0 24 24">
          <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M15 17h5l-1.405-1.405A2.032 2.032 0 0118 14.158V11a6.002 6.002 0 00-4-5.659V5a2 2 0 10-4 0v.341C7.67 6.165 6 8.388 6 11v3.159c0 .538-.214 1.055-.595 1.436L4 17h5m6 0v1a3 3 0 11-6 0v-1m6 0H9" />
        </svg>
        @if (alerts().length > 0) {
          <span class="absolute -top-0.5 -right-0.5 min-w-[14px] h-[14px] rounded-full bg-error text-white text-[9px] flex items-center justify-center font-bold leading-none px-0.5">
            {{ alerts().length }}
          </span>
        }
      </button>
      @if (open()) {
        <div class="absolute right-0 top-full mt-1.5 w-72 bg-surface-light border border-border rounded-lg shadow-2xl z-50 overflow-hidden">
          <div class="px-3 py-2 border-b border-border flex items-center justify-between">
            <span class="text-xs font-semibold text-text-muted uppercase tracking-wider">Health Alerts</span>
            @if (alerts().length > 0) {
              <span class="text-[10px] text-text-muted">{{ alerts().length }} active</span>
            }
          </div>
          @if (alerts().length > 0) {
            <div class="max-h-64 overflow-y-auto divide-y divide-border">
              @for (alert of alerts(); track $index) {
                <div class="px-3 py-2">
                  <div class="flex items-center gap-2">
                    <span [class]="severityClass(alert)">{{ severityLabel(alert) }}</span>
                    <span class="text-[10px] text-text font-mono">{{ formatCode(alert.code) }}</span>
                  </div>
                  @if (alert.description) {
                    <div class="text-[10px] text-text-muted mt-0.5 leading-relaxed">{{ alert.description }}</div>
                  }
                </div>
              }
            </div>
          } @else {
            <div class="px-3 py-6 text-center">
              <div class="text-xs text-success">All systems healthy</div>
              <div class="text-[10px] text-text-muted mt-0.5">No active alerts</div>
            </div>
          }
        </div>
      }
    </div>
  `,
})
export class HmsPanelComponent {
  alerts = input<HmsAlert[]>([]);
  open = signal(false);

  private el = inject(ElementRef);

  toggle(event: Event) {
    event.stopPropagation();
    this.open.update(v => !v);
  }

  @HostListener('document:click', ['$event'])
  onDocumentClick(event: Event) {
    if (this.open() && !this.el.nativeElement.contains(event.target)) {
      this.open.set(false);
    }
  }

  severityClass(alert: HmsAlert): string {
    const level = (alert.attr >> 16) & 0xff;
    const base = 'px-1 py-0.5 rounded text-[10px] font-medium';
    if (level >= 3) return `${base} bg-error/20 text-error`;
    if (level === 2) return `${base} bg-warning/20 text-warning`;
    return `${base} bg-primary/20 text-primary`;
  }

  severityLabel(alert: HmsAlert): string {
    const level = (alert.attr >> 16) & 0xff;
    if (level >= 3) return 'Error';
    if (level === 2) return 'Warning';
    return 'Info';
  }

  formatCode(code: number): string {
    return code.toString(16).toUpperCase().padStart(8, '0');
  }
}
