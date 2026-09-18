import { AfterViewInit, Component, inject } from '@angular/core';
import { FormGroup, FormControl, Validators } from '@angular/forms';
import { MatDialogTitle, MatDialogContent, MatDialogRef } from '@angular/material/dialog';
import { SharedModule } from '../../../../shared/shared.module';
import { GenerateWalletsComponent } from '../generate-wallets/generate-wallets.component';

@Component({
  selector: 'app-import-wallets',
  imports: [SharedModule, MatDialogTitle, MatDialogContent],
  templateUrl: './import-wallets.component.html',
  styleUrl: './import-wallets.component.scss'
})
export class ImportWalletsComponent implements AfterViewInit {
  importForm = new FormGroup({
    name: new FormControl('', [Validators.required]),
    comments: new FormControl(''),
    pks: new FormControl('', [Validators.required]),
  })
  readonly dialogRef = inject(MatDialogRef<GenerateWalletsComponent>);
  ngAfterViewInit(): void {

  }

  public importWallets() {
    let { pks, name, comments } = this.importForm.value;
    let payload = {
      name,
      comments,
      pks
    }
    this.dialogRef.close(payload)
  }
}
