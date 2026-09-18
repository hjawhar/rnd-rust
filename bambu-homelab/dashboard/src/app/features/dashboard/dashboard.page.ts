import { Component, inject, signal, OnInit, OnDestroy } from '@angular/core';
import { RouterLink } from '@angular/router';
import { Subscription } from 'rxjs';
import { PrinterService } from '../../core/services/printer.service';
import { WebSocketService } from '../../core/services/websocket.service';
import { TelemetryStore } from '../../core/services/telemetry.store';
import { AuthService } from '../../core/services/auth.service';
import { PrinterCardComponent } from './printer-card.component';
import { PrinterWithStatus } from '../../core/models/printer.model';

@Component({
  selector: 'app-dashboard',
  imports: [RouterLink, PrinterCardComponent],
  template: `
    <div class="p-6">
      <div class="flex items-center justify-between mb-6">
        <h1 class="text-2xl font-bold text-text">Printers</h1>
        @if (auth.isAdmin()) {
          <a routerLink="/printers/add"
             class="px-4 py-2 bg-primary text-white rounded-lg hover:bg-primary-dark transition-colors text-sm font-medium">
            + Add Printer
          </a>
        }
      </div>

      @if (printers().length === 0) {
        <div class="text-center py-16">
          @if (auth.isAdmin()) {
            <p class="text-text-muted text-lg">No printers registered yet.</p>
            <a routerLink="/printers/add" class="text-primary hover:text-primary-light mt-2 inline-block">Add your first printer</a>
          } @else {
            <p class="text-text-muted text-lg">No printers assigned to you yet.</p>
          }
        </div>
      } @else {
        <div class="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-4">
          @for (printer of printers(); track printer.id) {
            <app-printer-card [printer]="printer" />
          }
        </div>
      }
    </div>
  `,
})
export class DashboardPage implements OnInit, OnDestroy {
  private printerService = inject(PrinterService);
  private ws = inject(WebSocketService);
  private telemetryStore = inject(TelemetryStore);
  auth = inject(AuthService);

  printers = signal<PrinterWithStatus[]>([]);
  private subs: Subscription[] = [];

  ngOnInit() {
    this.loadPrinters();

    this.subs.push(
      this.ws.assignmentAdded$.subscribe(event => {
        const newPrinter: PrinterWithStatus = {
          id: event.printer_id,
          name: event.printer_name,
          ip: '',
          serial: event.printer_id,
          access_code: '',
          model: event.printer_model,
          online: false,
          status: null,
        };
        this.printers.update(list => [...list, newPrinter]);
        this.ws.subscribe([event.printer_id]);
      }),
      this.ws.assignmentRemoved$.subscribe(event => {
        this.printers.update(list => list.filter(p => p.id !== event.printer_id));
        this.ws.unsubscribe([event.printer_id]);
      }),
    );
  }

  ngOnDestroy() {
    this.subs.forEach(s => s.unsubscribe());
  }

  private loadPrinters() {
    this.printerService.listPrinters().subscribe({
      next: (printers) => {
        this.printers.set(printers);
        const ids = printers.map(p => p.id);
        if (ids.length > 0) {
          this.ws.subscribe(ids);
        }
      },
      error: (err) => console.error('Failed to load printers:', err),
    });
  }
}
