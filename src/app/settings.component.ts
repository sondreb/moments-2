import { CommonModule } from "@angular/common";
import { Component, EventEmitter, Input, Output } from "@angular/core";

export interface AiModelInfo {
  id: string;
  name: string;
  task: string;
  accelerator: string;
  description: string;
  fileName: string;
  downloadUrl: string;
  installed: boolean;
  sizeBytes: number;
}

export interface DatabaseStats {
  path: string;
  sizeBytes: number;
  rootCount: number;
  mediaCount: number;
  metadataCount: number;
  favoriteCount: number;
  tagCount: number;
  faceCount: number;
}

export interface AvailableUpdateInfo {
  currentVersion: string;
  version: string;
  date: string | null;
  body: string | null;
}

@Component({
  selector: "app-settings",
  imports: [CommonModule],
  templateUrl: "./settings.component.html",
  styleUrl: "./settings.component.css",
})
export class SettingsComponent {
  @Input({ required: true }) models: AiModelInfo[] = [];
  @Input() databaseStats: DatabaseStats | null = null;
  @Input() isInstallingModel = false;
  @Input() isDeletingModel = false;
  @Input() isClearingCache = false;
  @Input() isAnalyzingFolder = false;
  @Input() selectedRootName = "No folder selected";
  @Input() selectedRootId: string | null = null;
  @Input() themeMode: "auto" | "light" | "dark" = "auto";
  @Input() statusMessage = "";
  @Input() updaterEnabled = false;
  @Input() currentVersion = "";
  @Input() availableUpdate: AvailableUpdateInfo | null = null;
  @Input() updateStatusMessage = "";
  @Input() isCheckingForUpdates = false;
  @Input() isInstallingUpdate = false;
  @Input() updateDownloadedBytes = 0;
  @Input() updateContentLength = 0;
  @Input() updateReadyToRestart = false;

  @Output() installModel = new EventEmitter<string>();
  @Output() deleteModel = new EventEmitter<string>();
  @Output() analyzeRootFaces = new EventEmitter<void>();
  @Output() classifyRootImages = new EventEmitter<void>();
  @Output() themeModeChange = new EventEmitter<"auto" | "light" | "dark">();
  @Output() clearCache = new EventEmitter<void>();
  @Output() refresh = new EventEmitter<void>();
  @Output() checkForUpdates = new EventEmitter<void>();
  @Output() installUpdate = new EventEmitter<void>();
  @Output() restartToApplyUpdate = new EventEmitter<void>();
  @Output() closeSettings = new EventEmitter<void>();

  protected formatBytes(bytes: number): string {
    if (bytes < 1024) {
      return `${bytes} B`;
    }
    if (bytes < 1024 * 1024) {
      return `${(bytes / 1024).toFixed(1)} KB`;
    }
    if (bytes < 1024 * 1024 * 1024) {
      return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
    }
    return `${(bytes / 1024 / 1024 / 1024).toFixed(1)} GB`;
  }

  protected updateProgressPercent(): number {
    if (this.updateContentLength <= 0) {
      return 0;
    }

    return Math.max(0, Math.min(100, Math.round((this.updateDownloadedBytes / this.updateContentLength) * 100)));
  }
}
