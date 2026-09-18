import { Injectable, signal, computed, inject, DestroyRef } from '@angular/core';
import { takeUntilDestroyed } from '@angular/core/rxjs-interop';
import { WebSocketService } from './websocket.service';
import { TelemetrySnapshot, Temperature } from '../models/printer.model';

interface TempHistory {
  timestamps: number[];
  nozzle: number[];
  bed: number[];
  chamber: number[];
}

const MAX_HISTORY = 360; // 30 minutes at ~5s intervals

@Injectable({ providedIn: 'root' })
export class TelemetryStore {
  private ws = inject(WebSocketService);
  private destroyRef = inject(DestroyRef);

  private _snapshots = signal<Map<string, TelemetrySnapshot>>(new Map());
  private _tempHistory = signal<Map<string, TempHistory>>(new Map());

  readonly snapshots = this._snapshots.asReadonly();
  readonly tempHistory = this._tempHistory.asReadonly();
  readonly printerIds = computed(() => Array.from(this._snapshots().keys()));

  constructor() {
    this.ws.telemetry$
      .pipe(takeUntilDestroyed(this.destroyRef))
      .subscribe(msg => {
        // Update snapshot
        this._snapshots.update(map => {
          const next = new Map(map);
          next.set(msg.printer_id, msg.data);
          return next;
        });

        // Update temp history
        this._tempHistory.update(map => {
          const next = new Map(map);
          const history = next.get(msg.printer_id) ?? {
            timestamps: [], nozzle: [], bed: [], chamber: []
          };

          history.timestamps.push(Date.now());
          history.nozzle.push(msg.data.nozzle_temp?.current ?? 0);
          history.bed.push(msg.data.bed_temp?.current ?? 0);
          history.chamber.push(msg.data.chamber_temp?.current ?? 0);

          // Trim to max size
          if (history.timestamps.length > MAX_HISTORY) {
            history.timestamps = history.timestamps.slice(-MAX_HISTORY);
            history.nozzle = history.nozzle.slice(-MAX_HISTORY);
            history.bed = history.bed.slice(-MAX_HISTORY);
            history.chamber = history.chamber.slice(-MAX_HISTORY);
          }

          next.set(msg.printer_id, history);
          return next;
        });
      });
  }

  getSnapshot(printerId: string): TelemetrySnapshot | undefined {
    return this._snapshots().get(printerId);
  }

  getHistory(printerId: string): TempHistory | undefined {
    return this._tempHistory().get(printerId);
  }
}
