import { Component, input, inject, signal, OnInit } from '@angular/core';
import { FormsModule } from '@angular/forms';
import { HttpClient } from '@angular/common/http';

interface FileEntry {
  name: string;
}

@Component({
  selector: 'app-print-panel',
  imports: [FormsModule],
  template: `
    <div class="bg-surface rounded-lg p-3">
      <h3 class="text-xs font-medium text-text-muted uppercase tracking-wider mb-2">Files & Printing</h3>

      <!-- Upload -->
      <div class="mb-4">
        <label class="block text-xs text-text-muted mb-1">Upload to SD Card</label>
        <div class="flex gap-2">
          <input type="file" (change)="onFileSelected($event)" accept=".3mf,.gcode"
                 class="flex-1 text-sm text-text file:mr-2 file:py-1 file:px-3 file:rounded file:border-0 file:text-sm file:bg-surface-lighter file:text-text-muted hover:file:bg-primary hover:file:text-white" />
          <button (click)="uploadFile()" [disabled]="!selectedFile || uploading()"
                  class="px-3 py-1 bg-primary text-white rounded text-sm disabled:opacity-40">
            {{ uploading() ? 'Uploading...' : 'Upload' }}
          </button>
        </div>
        @if (uploadError()) {
          <div class="text-xs text-error mt-1">{{ uploadError() }}</div>
        }
        @if (uploadSuccess()) {
          <div class="text-xs text-success mt-1">Uploaded successfully</div>
        }
      </div>

      <!-- File list -->
      <div class="mb-3">
        <div class="flex items-center justify-between mb-2">
          <span class="text-xs text-text-muted">SD Card Files</span>
          <button (click)="loadFiles()" class="text-xs text-primary hover:text-primary-light">Refresh</button>
        </div>
        @if (loading()) {
          <div class="text-xs text-text-muted">Loading...</div>
        } @else if (files().length === 0) {
          <div class="text-xs text-text-muted">No files found</div>
        } @else {
          <div class="space-y-1 max-h-48 overflow-y-auto">
            @for (file of files(); track file.name) {
              <div class="flex items-center justify-between p-2 rounded bg-surface hover:bg-surface-lighter group">
                <span class="text-sm text-text truncate flex-1">{{ file.name }}</span>
                <button (click)="startPrint(file.name)"
                        class="px-2 py-1 bg-primary/20 text-primary rounded text-xs opacity-0 group-hover:opacity-100 transition-opacity">
                  Print
                </button>
              </div>
            }
          </div>
        }
      </div>

      @if (printSuccess()) {
        <div class="text-xs text-success">Print command sent!</div>
      }
    </div>
  `,
})
export class PrintPanelComponent implements OnInit {
  printerId = input.required<string>();
  private http = inject(HttpClient);

  files = signal<FileEntry[]>([]);
  loading = signal(false);
  selectedFile: File | null = null;
  uploading = signal(false);
  uploadError = signal<string | null>(null);
  uploadSuccess = signal(false);
  printSuccess = signal(false);

  ngOnInit() {
    this.loadFiles();
  }

  loadFiles() {
    this.loading.set(true);
    this.http.get<FileEntry[]>(`/api/printers/${this.printerId()}/files`).subscribe({
      next: (files) => { this.files.set(files); this.loading.set(false); },
      error: () => { this.loading.set(false); },
    });
  }

  onFileSelected(event: Event) {
    const input = event.target as HTMLInputElement;
    this.selectedFile = input.files?.[0] ?? null;
  }

  uploadFile() {
    if (!this.selectedFile) return;
    this.uploading.set(true);
    this.uploadError.set(null);
    this.uploadSuccess.set(false);

    const formData = new FormData();
    formData.append('file', this.selectedFile);

    this.http.post(`/api/printers/${this.printerId()}/upload`, formData).subscribe({
      next: () => {
        this.uploading.set(false);
        this.uploadSuccess.set(true);
        this.loadFiles();
        setTimeout(() => this.uploadSuccess.set(false), 3000);
      },
      error: (err) => {
        this.uploading.set(false);
        this.uploadError.set(err.error || 'Upload failed');
      },
    });
  }

  startPrint(filename: string) {
    this.printSuccess.set(false);
    this.http.post(`/api/printers/${this.printerId()}/print`, { filename, plate: 1 }).subscribe({
      next: () => {
        this.printSuccess.set(true);
        setTimeout(() => this.printSuccess.set(false), 3000);
      },
    });
  }
}
