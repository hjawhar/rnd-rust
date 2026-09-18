import { Component, inject } from '@angular/core';
import { MatDialogTitle, MatDialogContent, MatDialogRef } from '@angular/material/dialog';
import { SharedModule } from '../../../../../shared/shared.module';

@Component({
  selector: 'app-confirm-delete-project',
  imports: [SharedModule, MatDialogTitle, MatDialogContent],
  templateUrl: './confirm-delete-project.component.html',
  styleUrl: './confirm-delete-project.component.scss'
})
export class ConfirmDeleteProjectComponent {
  readonly dialogRef = inject(MatDialogRef<ConfirmDeleteProjectComponent>);
  ngAfterViewInit(): void {

  }
}
