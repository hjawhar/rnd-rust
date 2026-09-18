import { Component, inject } from '@angular/core';
import { MatDialogTitle, MatDialogContent, MatDialogRef, MAT_DIALOG_DATA } from '@angular/material/dialog';
import { SharedModule } from '../../shared.module';

@Component({
  selector: 'app-confirm-action',
  imports: [SharedModule, MatDialogTitle, MatDialogContent],
  templateUrl: './confirm-action.component.html',
  styleUrl: './confirm-action.component.scss'
})
export class ConfirmActionComponent {
  readonly dialogRef = inject(MatDialogRef<ConfirmActionComponent>);
  public data = inject<{ title: string, action: string }>(MAT_DIALOG_DATA);

  ngAfterViewInit(): void {

  }
}
