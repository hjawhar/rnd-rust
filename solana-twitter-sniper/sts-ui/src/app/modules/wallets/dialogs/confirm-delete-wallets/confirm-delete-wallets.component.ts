import { Component, inject } from '@angular/core';
import { MatDialogTitle, MatDialogContent, MatDialogRef } from '@angular/material/dialog';
import { SharedModule } from '../../../../shared/shared.module';

@Component({
  selector: 'app-confirm-delete-wallets',
  imports: [SharedModule, MatDialogTitle, MatDialogContent],
  templateUrl: './confirm-delete-wallets.component.html',
  styleUrl: './confirm-delete-wallets.component.scss'
})
export class ConfirmDeleteWalletsComponent {
  readonly dialogRef = inject(MatDialogRef<ConfirmDeleteWalletsComponent>);
  ngAfterViewInit(): void {

  }
}
