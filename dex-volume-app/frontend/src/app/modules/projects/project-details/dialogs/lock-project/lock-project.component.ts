import { Component, inject } from '@angular/core';
import { MatDialogTitle, MatDialogContent, MatDialogRef } from '@angular/material/dialog';
import { SharedModule } from '../../../../../shared/shared.module';
import { FormControl } from '@angular/forms';

export type LockProjectResult = { mode: 'immediate' } | { mode: 'scheduled', lockAt: number };

@Component({
  selector: 'app-lock-project',
  imports: [SharedModule, MatDialogTitle, MatDialogContent],
  templateUrl: './lock-project.component.html',
  styleUrl: './lock-project.component.scss'
})
export class LockProjectComponent {
  readonly dialogRef = inject(MatDialogRef<LockProjectComponent>);
  mode: 'immediate' | 'scheduled' = 'immediate';
  datetimeControl = new FormControl<string>('');

  get minDatetime(): string {
    return new Date().toISOString().slice(0, 16);
  }

  confirm() {
    if (this.mode === 'immediate') {
      this.dialogRef.close({ mode: 'immediate' } as LockProjectResult);
    } else {
      const value = this.datetimeControl.value;
      if (!value) return;
      const lockAt = Math.floor(new Date(value).getTime() / 1000);
      if (lockAt <= Math.floor(Date.now() / 1000)) return;
      this.dialogRef.close({ mode: 'scheduled', lockAt } as LockProjectResult);
    }
  }
}
