import { CommonModule, DatePipe, DecimalPipe } from "@angular/common";
import { NgModule, ModuleWithProviders } from "@angular/core";
import { MatDatepickerModule } from "@angular/material/datepicker";
import { MatIconModule } from "@angular/material/icon";
import { AuthInterceptor } from "./interceptors/authinterceptor";
import { HTTP_INTERCEPTORS } from "@angular/common/http";
import { FormsModule, ReactiveFormsModule } from "@angular/forms";
import { MatFormFieldModule } from "@angular/material/form-field";
import { MatTableModule } from "@angular/material/table";
import { MatSnackBarModule } from "@angular/material/snack-bar";
import { MatSelectModule } from "@angular/material/select";
import { MatTooltipModule } from "@angular/material/tooltip";
import { MatDialogModule } from "@angular/material/dialog";
import { MatInputModule } from "@angular/material/input";
import { MatButtonModule } from "@angular/material/button";
import { MatCheckboxModule } from "@angular/material/checkbox";
import { MatTabsModule } from "@angular/material/tabs";
import { MatPaginatorModule } from "@angular/material/paginator";
import { MatButtonToggleModule } from "@angular/material/button-toggle";
import { ClipboardModule } from "@angular/cdk/clipboard";
import { MatExpansionModule } from "@angular/material/expansion";
import { MatSliderModule } from "@angular/material/slider";
import { MatSidenavModule } from "@angular/material/sidenav";
import { MatDividerModule } from "@angular/material/divider";
import { MatSortModule } from "@angular/material/sort";
import { MatMenuModule } from "@angular/material/menu";
import { MatGridListModule } from "@angular/material/grid-list";
import { MatSlideToggleModule } from '@angular/material/slide-toggle';
import { MatToolbarModule } from '@angular/material/toolbar';
import { MatChipInputEvent, MatChipsModule } from '@angular/material/chips';

@NgModule({
  declarations: [
  ],
  imports: [
    CommonModule,
    MatIconModule,
    MatDatepickerModule,
    ReactiveFormsModule,
    MatFormFieldModule,
    MatInputModule,
    MatButtonModule,
    MatButtonToggleModule,
    MatTableModule,
    MatSnackBarModule,
    MatSelectModule,
    MatTooltipModule,
    MatDialogModule,
    MatCheckboxModule,
    MatTabsModule,
    MatPaginatorModule,
    FormsModule,
    ClipboardModule,
    MatExpansionModule,
    MatSliderModule,
    MatSidenavModule,
    MatMenuModule,
    MatSortModule,
    MatSortModule,
    MatInputModule,
    MatMenuModule,
    MatDividerModule,
    MatGridListModule,
    MatSlideToggleModule,
    MatToolbarModule,
    MatChipsModule
  ],
  exports: [
    MatIconModule,
    MatDatepickerModule,
    ReactiveFormsModule,
    MatFormFieldModule,
    MatInputModule,
    MatButtonModule,
    MatButtonToggleModule,
    MatTableModule,
    MatSnackBarModule,
    MatSelectModule,
    MatTooltipModule,
    MatDialogModule,
    MatCheckboxModule,
    MatTabsModule,
    MatPaginatorModule,
    FormsModule,
    ClipboardModule,
    MatExpansionModule,
    MatSliderModule,
    MatSidenavModule,
    MatMenuModule,
    MatDividerModule,
    MatSortModule,
    MatGridListModule,
    MatSlideToggleModule,
    MatToolbarModule,
    MatChipsModule
  ],
})
export class SharedModule {
  static forRoot(): ModuleWithProviders<SharedModule> {
    return {
      ngModule: SharedModule,
      providers: [
        { provide: HTTP_INTERCEPTORS, useClass: AuthInterceptor, multi: true },
        DecimalPipe,
        DatePipe
      ],
    };
  }
}
