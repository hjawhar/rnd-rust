import { Component, inject, signal, computed, OnInit, OnDestroy, HostListener } from '@angular/core';
import { ActivatedRoute, Router, RouterLink } from '@angular/router';
import { Subscription } from 'rxjs';
import { PrinterService } from '../../core/services/printer.service';
import { WebSocketService } from '../../core/services/websocket.service';
import { TelemetryStore } from '../../core/services/telemetry.store';
import { PrinterWithStatus, TelemetrySnapshot } from '../../core/models/printer.model';
import { AuthService } from '../../core/services/auth.service';
import { StatusBadgeComponent } from '../../shared/components/status-badge.component';
import { TemperaturePanelComponent } from './temperature-panel.component';
import { PrintProgressComponent } from './print-progress.component';
import { AmsPanelComponent } from './ams-panel.component';
import { InfoPanelComponent } from './info-panel.component';
import { ControlPanelComponent } from './control-panel.component';
import { VersionPanelComponent } from './version-panel.component';
import { HmsPanelComponent } from './hms-panel.component';
import { CameraPanelComponent } from './camera-panel.component';
import { HistoryPanelComponent } from './history-panel.component';
import { StatsPanelComponent } from './stats-panel.component';
import { PrintPanelComponent } from './print-panel.component';
import { QueuePanelComponent } from './queue-panel.component';
import { GcodePreviewComponent } from './gcode-preview.component';
import { CommandRequest } from '../../core/models/command.model';

@Component({
  selector: 'app-printer-detail',
  imports: [
    RouterLink, StatusBadgeComponent, TemperaturePanelComponent,
    PrintProgressComponent, AmsPanelComponent, InfoPanelComponent,
    ControlPanelComponent, VersionPanelComponent, HmsPanelComponent, CameraPanelComponent, HistoryPanelComponent, StatsPanelComponent, PrintPanelComponent, QueuePanelComponent, GcodePreviewComponent,
  ],
  template: `
    <div class="p-4 space-y-3">
      @if (printer(); as p) {
        <!-- Header -->
        <div class="flex items-center justify-between">
          <div class="flex items-center gap-2.5">
            <a routerLink="/" class="text-xs text-text-muted hover:text-primary transition-colors">&larr; Printers</a>
            <div class="w-px h-4 bg-border"></div>
            <h1 class="text-lg font-semibold text-text">{{ p.name }}</h1>
            <app-status-badge [state]="snapshot()?.state ?? 0" [online]="p.online" />
            <span class="text-[11px] text-text-muted font-mono hidden sm:inline">{{ p.model }} &bull; {{ p.serial }}</span>
          </div>
          <div class="flex items-center gap-3">
            @if (snapshot()) {
              <span class="flex items-center gap-1.5 text-[10px] text-text-muted">
                <span class="w-1.5 h-1.5 rounded-full bg-success animate-pulse"></span>
                Live
              </span>
            }
            @if (snapshot(); as s) {
              <app-hms-panel [alerts]="s.hms_alerts" />
            }
          </div>
        </div>

        @if (snapshot(); as s) {
          <!-- Main: Camera + Sidebar -->
          <div class="grid grid-cols-1 lg:grid-cols-5 gap-3">
            <!-- Camera (3/5 width) — absolute so sidebar dictates row height -->
            <div class="lg:col-span-3 relative">
              <app-camera-panel [rtspUrl]="s.rtsp_url" [printerId]="printerId" [readOnly]="!auth.isAdmin()" class="block lg:absolute lg:inset-0" />
            </div>
            <!-- Right sidebar: stacked real-time panels -->
            <div class="lg:col-span-2 flex flex-col gap-3">
              <app-print-progress [snapshot]="s" />
              <app-temperature-panel [snapshot]="s" [printerId]="printerId" />
              <app-ams-panel [ams]="s.ams" />
              @if (auth.isAdmin()) {
                <app-control-panel [snapshot]="s" (onCommand)="handleCommand($event)" />
              }
            </div>
          </div>

          <!-- Tabbed secondary section -->
          <div class="bg-surface-light rounded-lg border border-border">
            <div class="flex border-b border-border">
              @for (tab of tabs(); track tab.id) {
                <button (click)="activeTab.set(tab.id)"
                        [class]="activeTab() === tab.id
                          ? 'text-primary border-b-2 border-primary bg-surface-lighter/30'
                          : 'text-text-muted hover:text-text hover:bg-surface-lighter/20'"
                        class="px-4 py-2 text-xs font-medium transition-colors whitespace-nowrap -mb-px">
                  {{ tab.label }}
                </button>
              }
            </div>
            <div class="p-3">
              @switch (activeTab()) {
                @case ('info') {
                  <div class="grid grid-cols-1 lg:grid-cols-2 gap-3">
                    <app-info-panel [snapshot]="s" />
                    <app-version-panel [snapshot]="s" [readOnly]="!auth.isAdmin()" (onUpgrade)="handleUpgrade()" />
                  </div>
                }
                @case ('files') { <app-print-panel [printerId]="printerId" /> }
                @case ('queue') { <app-queue-panel [printerId]="printerId" /> }
                @case ('history') { <app-history-panel [printerId]="printerId" /> }
                @case ('stats') { <app-stats-panel [printerId]="printerId" [snapshot]="snapshot()" /> }
                @case ('gcode') { <app-gcode-preview /> }
              }
            </div>
          </div>

          @if (auth.isAdmin()) {
            <!-- Keyboard shortcuts -->
            <div class="text-[10px] text-text-muted/40 text-center">
              Space=pause/resume &bull; S=stop &bull; 1-4=speed &bull; L=light
            </div>
          }
        } @else {
          <div class="flex items-center justify-center h-64 text-sm text-text-muted">
            <div class="flex items-center gap-2">
              <span class="w-2 h-2 rounded-full bg-primary animate-pulse"></span>
              Waiting for telemetry...
            </div>
          </div>
        }
      } @else {
        <div class="flex items-center justify-center h-64 text-sm text-text-muted">Loading printer...</div>
      }
    </div>
  `,
})
export class PrinterDetailPage implements OnInit, OnDestroy {
  private route = inject(ActivatedRoute);
  private router = inject(Router);
  private printerService = inject(PrinterService);
  private ws = inject(WebSocketService);
  private telemetryStore = inject(TelemetryStore);
  auth = inject(AuthService);
  private sub?: Subscription;

  printer = signal<PrinterWithStatus | null>(null);
  printerId = '';

  activeTab = signal('info');
  tabs = computed(() => {
    if (this.auth.isAdmin()) {
      return [
        { id: 'info', label: 'Info & Firmware' },
        { id: 'files', label: 'Files' },
        { id: 'queue', label: 'Queue' },
        { id: 'history', label: 'History' },
        { id: 'stats', label: 'Statistics' },
        { id: 'gcode', label: 'G-code' },
      ];
    }
    return [
      { id: 'info', label: 'Info & Firmware' },
      { id: 'history', label: 'History' },
      { id: 'stats', label: 'Statistics' },
    ];
  });

  snapshot = computed(() => {
    const ws = this.telemetryStore.getSnapshot(this.printerId);
    return ws ?? this.printer()?.status ?? null;
  });

  ngOnInit() {
    this.printerId = this.route.snapshot.params['id'];
    this.printerService.getPrinter(this.printerId).subscribe({
      next: (p) => this.printer.set(p),
      error: (err) => console.error('Failed to load printer:', err),
    });
    this.ws.subscribe([this.printerId]);
    this.sub = this.ws.assignmentRemoved$.subscribe(event => {
      if (event.printer_id === this.printerId) {
        this.router.navigate(['/']);
      }
    });
  }

  ngOnDestroy() {
    this.ws.unsubscribe([this.printerId]);
    this.sub?.unsubscribe();
  }

  handleCommand(cmd: CommandRequest) {
    this.printerService.sendCommand(this.printerId, cmd).subscribe({
      error: (err: unknown) => console.error('Command failed:', err),
    });
  }

  handleUpgrade() {
    // TODO: call upgrade endpoint when implemented
    console.log('Upgrade requested for', this.printerId);
  }

  @HostListener('document:keydown', ['$event'])
  onKeydown(event: KeyboardEvent) {
    if (!this.auth.isAdmin()) return;
    const target = event.target as HTMLElement;
    if (target.tagName === 'INPUT' || target.tagName === 'TEXTAREA' || target.tagName === 'SELECT') return;

    const s = this.snapshot();
    if (!s) return;

    switch (event.key) {
      case ' ':
        event.preventDefault();
        if (s.state === 3) {
          this.handleCommand({ command: 'resume' });
        } else if (s.state === 2) {
          this.handleCommand({ command: 'pause' });
        }
        break;
      case 's':
        if (s.state === 2 || s.state === 3 || s.state === 5) {
          if (confirm('Stop the print?')) {
            this.handleCommand({ command: 'stop' });
          }
        }
        break;
      case '1':
        this.handleCommand({ command: 'set_speed', params: { level: 1 } });
        break;
      case '2':
        this.handleCommand({ command: 'set_speed', params: { level: 2 } });
        break;
      case '3':
        this.handleCommand({ command: 'set_speed', params: { level: 3 } });
        break;
      case '4':
        this.handleCommand({ command: 'set_speed', params: { level: 4 } });
        break;
      case 'l': {
        const chamberLight = s.lights?.find(l => l.node === 'chamber_light');
        const mode = chamberLight?.mode === 'on' ? 'off' : 'on';
        this.handleCommand({ command: 'set_light', params: { node: 'chamber_light', mode } });
        break;
      }
    }
  }
}
