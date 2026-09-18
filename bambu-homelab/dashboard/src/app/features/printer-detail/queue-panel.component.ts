import { Component, input, inject, signal, OnInit } from '@angular/core';
import { FormsModule } from '@angular/forms';
import { HttpClient } from '@angular/common/http';

interface QueueItem {
  id: string;
  file_name: string;
  plate_number: number;
  status: string;
  position: number;
}

@Component({
  selector: 'app-queue-panel',
  imports: [FormsModule],
  template: `
    <div class="bg-surface rounded-lg p-3">
      <h3 class="text-xs font-medium text-text-muted uppercase tracking-wider mb-2">Print Queue</h3>

      <!-- Add to queue -->
      <div class="flex gap-2 mb-3">
        <input type="text" [(ngModel)]="newFileName" name="queueFile" placeholder="filename.3mf"
               class="flex-1 px-2 py-1.5 bg-surface-lighter border border-border rounded text-sm text-text placeholder-text-muted/50 focus:border-primary focus:outline-none" />
        <button (click)="addToQueue()" [disabled]="!newFileName"
                class="px-3 py-1.5 bg-primary text-white rounded text-sm disabled:opacity-40">
          + Queue
        </button>
      </div>

      <!-- Queue list -->
      @if (items().length > 0) {
        <div class="space-y-1">
          @for (item of items(); track item.id) {
            <div class="flex items-center justify-between p-2 rounded bg-surface">
              <div class="flex-1">
                <span class="text-sm text-text">{{ item.file_name }}</span>
                <span class="text-xs text-text-muted ml-2">#{{ item.position + 1 }}</span>
              </div>
              <button (click)="removeFromQueue(item.id)"
                      class="text-xs text-error hover:text-error/80 px-2">
                Remove
              </button>
            </div>
          }
        </div>
      } @else {
        <div class="text-xs text-text-muted">Queue is empty</div>
      }
    </div>
  `,
})
export class QueuePanelComponent implements OnInit {
  printerId = input.required<string>();
  private http = inject(HttpClient);

  items = signal<QueueItem[]>([]);
  newFileName = '';

  ngOnInit() { this.loadQueue(); }

  loadQueue() {
    this.http.get<QueueItem[]>(`/api/printers/${this.printerId()}/queue`).subscribe({
      next: (items) => this.items.set(items),
    });
  }

  addToQueue() {
    if (!this.newFileName) return;
    this.http.post(`/api/printers/${this.printerId()}/queue`, { file_name: this.newFileName }).subscribe({
      next: () => { this.newFileName = ''; this.loadQueue(); },
    });
  }

  removeFromQueue(id: string) {
    this.http.delete(`/api/printers/${this.printerId()}/queue/${id}`).subscribe({
      next: () => this.loadQueue(),
    });
  }
}
