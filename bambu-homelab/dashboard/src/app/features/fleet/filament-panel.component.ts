import { Component, inject, signal, OnInit } from '@angular/core';
import { HttpClient } from '@angular/common/http';

interface FilamentInfo {
  tray_id: string;
  filament_type: string;
  color: string;
  remain_pct: number;
  sub_brand: string;
}

@Component({
  selector: 'app-filament-panel',
  template: `
    <div class="bg-surface-light rounded-lg border border-border p-4">
      <h3 class="text-sm font-semibold text-text-muted mb-3">Filament Inventory</h3>
      @if (filaments().length > 0) {
        <div class="grid grid-cols-2 md:grid-cols-4 gap-3">
          @for (f of filaments(); track f.tray_id) {
            <div class="bg-surface rounded-lg p-3 text-center">
              <div class="w-8 h-8 rounded-full mx-auto mb-2 border border-border"
                   [style.background-color]="'#' + f.color.slice(0, 6)"></div>
              <div class="text-sm text-text font-medium">{{ f.filament_type }}</div>
              @if (f.sub_brand) {
                <div class="text-xs text-text-muted">{{ f.sub_brand }}</div>
              }
              <div class="mt-1">
                <div class="w-full bg-surface rounded-full h-1.5">
                  <div class="rounded-full h-1.5 transition-all"
                       [class]="f.remain_pct > 50 ? 'bg-success' : f.remain_pct > 20 ? 'bg-warning' : 'bg-error'"
                       [style.width.%]="f.remain_pct"></div>
                </div>
                <div class="text-xs text-text-muted mt-0.5">{{ f.remain_pct }}%</div>
              </div>
              <div class="text-xs text-text-muted">Tray {{ f.tray_id }}</div>
            </div>
          }
        </div>
      } @else {
        <div class="text-sm text-text-muted">No filament data available</div>
      }
    </div>
  `,
})
export class FilamentPanelComponent implements OnInit {
  private http = inject(HttpClient);
  filaments = signal<FilamentInfo[]>([]);

  ngOnInit() {
    this.http.get<FilamentInfo[]>('/api/filament').subscribe({
      next: (f) => this.filaments.set(f),
    });
  }
}
