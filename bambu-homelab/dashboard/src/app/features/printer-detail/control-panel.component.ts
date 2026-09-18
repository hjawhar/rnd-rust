import { Component, input, output, computed } from '@angular/core';
import { TelemetrySnapshot } from '../../core/models/printer.model';

@Component({
  selector: 'app-control-panel',
  template: `
    <div class="bg-surface-light rounded-lg border border-border p-3">
      <div class="flex gap-1.5 mb-2">
        @if (isPaused()) {
          <button (click)="onCommand.emit({command: 'resume'})"
                  class="flex-1 px-2 py-1.5 bg-success/20 text-success rounded hover:bg-success/30 text-xs font-medium">
            ▶ Resume
          </button>
        } @else {
          <button (click)="onCommand.emit({command: 'pause'})"
                  [disabled]="!isPrinting()"
                  class="flex-1 px-2 py-1.5 bg-warning/20 text-warning rounded hover:bg-warning/30 text-xs font-medium disabled:opacity-40">
            ⏸ Pause
          </button>
        }
        <button (click)="confirmStop()"
                [disabled]="!isActive()"
                class="flex-1 px-2 py-1.5 bg-error/20 text-error rounded hover:bg-error/30 text-xs font-medium disabled:opacity-40">
          ■ Stop
        </button>
        <button (click)="onCommand.emit({command: 'home'})"
                [disabled]="isPrinting()"
                class="px-2 py-1.5 bg-surface-lighter text-text-muted rounded hover:text-text text-xs">
          ⌂ Home
        </button>
      </div>

      <div class="mb-2">
        <div class="text-[10px] text-text-muted mb-1">Speed</div>
        <div class="grid grid-cols-4 gap-1">
          @for (level of speedLevels; track level.value) {
            <button (click)="onCommand.emit({command: 'set_speed', params: {level: level.value}})"
                    [class]="level.value === currentSpeed() ? 'bg-primary text-white' : 'bg-surface-lighter text-text-muted hover:text-text'"
                    class="px-1.5 py-1 rounded text-[10px] font-medium transition-colors">
              {{ level.label }}
            </button>
          }
        </div>
      </div>

      <div>
        <div class="text-[10px] text-text-muted mb-1">Lights</div>
        <div class="flex gap-1.5">
          @for (light of lights(); track light.node) {
            <button (click)="toggleLight(light.node, light.mode)"
                    [class]="light.mode === 'on' ? 'bg-warning/20 text-warning' : 'bg-surface-lighter text-text-muted'"
                    class="px-2 py-1 rounded text-[10px] font-medium transition-colors">
              {{ light.node === 'chamber_light' ? 'Chamber' : 'Work' }}: {{ light.mode }}
            </button>
          }
        </div>
      </div>
    </div>
  `,
})
export class ControlPanelComponent {
  snapshot = input.required<TelemetrySnapshot>();
  onCommand = output<{ command: string; params?: Record<string, unknown> }>();

  speedLevels = [
    { value: 1, label: 'Silent' },
    { value: 2, label: 'Standard' },
    { value: 3, label: 'Sport' },
    { value: 4, label: 'Ludicrous' },
  ];

  isPrinting = computed(() => this.snapshot().state === 2);
  isPaused = computed(() => this.snapshot().state === 3);
  isActive = computed(() => [2, 3, 5].includes(this.snapshot().state));
  currentSpeed = computed(() => this.snapshot().speed_profile);
  lights = computed(() => this.snapshot().lights ?? []);

  confirmStop() {
    if (confirm('Are you sure you want to stop the print?')) {
      this.onCommand.emit({ command: 'stop' });
    }
  }

  toggleLight(node: string, currentMode: string) {
    const newMode = currentMode === 'on' ? 'off' : 'on';
    this.onCommand.emit({ command: 'set_light', params: { node, mode: newMode } });
  }
}
