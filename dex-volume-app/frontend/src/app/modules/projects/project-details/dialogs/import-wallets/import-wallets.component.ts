import { AfterViewInit, Component, inject } from '@angular/core';
import { FormGroup, FormControl, Validators } from '@angular/forms';
import { MatDialogTitle, MatDialogContent, MatDialogRef } from '@angular/material/dialog';
import { GenerateWalletsComponent } from '../generate-wallets/generate-wallets.component';
import { SharedModule } from '../../../../../shared/shared.module';

@Component({
  selector: 'app-import-wallets',
  imports: [SharedModule, MatDialogTitle, MatDialogContent],
  templateUrl: './import-wallets.component.html',
  styleUrl: './import-wallets.component.scss'
})
export class ImportWalletsComponent implements AfterViewInit {
  importForm = new FormGroup({
    pks: new FormControl('', [Validators.required]),
  })
  readonly dialogRef = inject(MatDialogRef<GenerateWalletsComponent>);
  ngAfterViewInit(): void {

  }

  public importWallets() {
    let { pks } = this.importForm.value;
    let payload = {
      pks
    }
    this.dialogRef.close(payload)
  }
}
