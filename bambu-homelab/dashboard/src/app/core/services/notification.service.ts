import { Injectable, inject, DestroyRef } from '@angular/core';
import { takeUntilDestroyed } from '@angular/core/rxjs-interop';
import { WebSocketService } from './websocket.service';

@Injectable({ providedIn: 'root' })
export class NotificationService {
  private ws = inject(WebSocketService);
  private destroyRef = inject(DestroyRef);
  private previousStates = new Map<string, string>();

  init(): void {
    if ('Notification' in window && Notification.permission === 'default') {
      Notification.requestPermission();
    }

    this.ws.telemetry$
      .pipe(takeUntilDestroyed(this.destroyRef))
      .subscribe(msg => {
        const printerId = msg.printer_id;
        const state = msg.data.gcode_state;
        const prev = this.previousStates.get(printerId);
        this.previousStates.set(printerId, state);

        if (!prev) return;

        if (prev === 'RUNNING' && state === 'FINISH') {
          this.notify('Print Complete', `${msg.data.subtask_name || 'Print job'} finished successfully.`);
        } else if (prev === 'RUNNING' && state === 'FAILED') {
          this.notify('Print Failed', `${msg.data.subtask_name || 'Print job'} failed.`);
        } else if (state === 'PAUSE' && prev === 'RUNNING') {
          this.notify('Print Paused', `${msg.data.subtask_name || 'Print job'} was paused.`);
        }
      });
  }

  private notify(title: string, body: string): void {
    if ('Notification' in window && Notification.permission === 'granted') {
      new Notification(title, { body, icon: '/favicon.ico' });
    }
  }
}
