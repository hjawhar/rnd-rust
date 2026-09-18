import { Component, input, computed } from '@angular/core';
import { printerStateLabel } from '../../core/models/printer.model';

@Component({
  selector: 'app-status-badge',
  template: `
    <span [class]="badgeClass()">
      {{ label() }}
    </span>
  `,
})
export class StatusBadgeComponent {
  state = input.required<number>();
  online = input(true);

  label = computed(() => {
    if (!this.online()) return 'Offline';
    return printerStateLabel(this.state());
  });

  badgeClass = computed(() => {
    if (!this.online()) return 'px-2 py-0.5 rounded-full text-xs font-medium bg-error/20 text-error';
    const state = this.state();
    const base = 'px-2 py-0.5 rounded-full text-xs font-medium';
    switch (state) {
      case 1: return `${base} bg-success/20 text-success`;  // Idle
      case 2: return `${base} bg-primary/20 text-primary`;  // Printing
      case 3: return `${base} bg-warning/20 text-warning`;  // Paused
      case 4: return `${base} bg-error/20 text-error`;      // Error
      case 5: return `${base} bg-primary/20 text-primary`;  // Preparing
      case 7: return `${base} bg-success/20 text-success`;  // Finished
      case 8: return `${base} bg-error/20 text-error`;      // Failed
      default: return `${base} bg-surface-lighter text-text-muted`;
    }
  });
}
