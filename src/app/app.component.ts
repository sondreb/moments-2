import { CommonModule } from "@angular/common";
import { Component, OnInit, computed, signal } from "@angular/core";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";

type LibraryRootStatus = "ready" | "scanning" | "error";

interface LibraryRoot {
  id: string;
  name: string;
  path: string;
  status: LibraryRootStatus;
  photoCount: number;
}

interface ScanStats {
  rootId: string;
  photoCount: number;
  skippedCount: number;
}

interface LibraryOverview {
  rootCount: number;
  photoCount: number;
}

@Component({
  selector: "app-root",
  imports: [CommonModule],
  templateUrl: "./app.component.html",
  styleUrl: "./app.component.css",
})
export class AppComponent implements OnInit {
  protected readonly roots = signal<LibraryRoot[]>([]);
  protected readonly overview = signal<LibraryOverview>({ rootCount: 0, photoCount: 0 });
  protected readonly selectedRootId = signal<string | null>(null);
  protected readonly isAddingFolder = signal(false);
  protected readonly lastScan = signal<ScanStats | null>(null);
  protected readonly errorMessage = signal<string | null>(null);

  protected readonly selectedRoot = computed(() => {
    const selectedRootId = this.selectedRootId();
    return this.roots().find((root) => root.id === selectedRootId) ?? this.roots()[0] ?? null;
  });

  protected readonly selectedRootName = computed(() => this.selectedRoot()?.name ?? "No folder selected");
  protected readonly selectedRootPath = computed(() => this.selectedRoot()?.path ?? "-");
  protected readonly selectedRootStatus = computed(() => this.selectedRoot()?.status ?? "ready");
  protected readonly selectedRootPhotoCount = computed(() => this.selectedRoot()?.photoCount ?? 0);
  protected readonly totalPhotos = computed(() => this.overview().photoCount.toLocaleString());

  async ngOnInit(): Promise<void> {
    await this.refreshLibrary();
  }

  protected async addFolder(): Promise<void> {
    this.isAddingFolder.set(true);
    this.errorMessage.set(null);

    try {
      const selection = await open({ directory: true, multiple: false, title: "Add folder to Moments" });

      if (typeof selection !== "string") {
        return;
      }

      const root = await invoke<LibraryRoot>("add_library_root", { path: selection });
      this.selectedRootId.set(root.id);
      await this.refreshLibrary();
    } catch (error) {
      this.errorMessage.set(this.describeError(error));
    } finally {
      this.isAddingFolder.set(false);
    }
  }

  protected async scanRoot(root: LibraryRoot): Promise<void> {
    this.errorMessage.set(null);
    this.selectedRootId.set(root.id);
    this.roots.update((roots) =>
      roots.map((candidate) =>
        candidate.id === root.id ? { ...candidate, status: "scanning" } : candidate,
      ),
    );

    try {
      const stats = await invoke<ScanStats>("scan_library_root", { rootId: root.id });
      this.lastScan.set(stats);
      await this.refreshLibrary();
    } catch (error) {
      this.errorMessage.set(this.describeError(error));
      await this.refreshLibrary();
    }
  }

  protected selectRoot(root: LibraryRoot): void {
    this.selectedRootId.set(root.id);
  }

  protected isSelected(root: LibraryRoot): boolean {
    return this.selectedRoot()?.id === root.id;
  }

  private async refreshLibrary(): Promise<void> {
    const [roots, overview] = await Promise.all([
      invoke<LibraryRoot[]>("list_library_roots"),
      invoke<LibraryOverview>("library_overview"),
    ]);

    this.roots.set(roots);
    this.overview.set(overview);
  }

  private describeError(error: unknown): string {
    return error instanceof Error ? error.message : String(error);
  }
}
