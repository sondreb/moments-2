import { CommonModule } from "@angular/common";
import { Component, HostListener, OnInit, computed, effect, signal } from "@angular/core";
import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { openPath } from "@tauri-apps/plugin-opener";

type LibraryRootStatus = "ready" | "scanning" | "error";
type MediaType = "photo" | "video";
type ThemeMode = "auto" | "light" | "dark";

interface LibraryRoot {
  id: string;
  name: string;
  path: string;
  status: LibraryRootStatus;
  photoCount: number;
  videoCount: number;
  mediaCount: number;
}

interface ScanStats {
  rootId: string;
  photoCount: number;
  videoCount: number;
  mediaCount: number;
  skippedCount: number;
}

interface MediaItem {
  id: string;
  rootId: string;
  name: string;
  path: string;
  mediaType: MediaType;
}

interface LibraryOverview {
  rootCount: number;
  photoCount: number;
  videoCount: number;
  mediaCount: number;
}

@Component({
  selector: "app-root",
  imports: [CommonModule],
  templateUrl: "./app.component.html",
  styleUrl: "./app.component.css",
})
export class AppComponent implements OnInit {
  protected readonly roots = signal<LibraryRoot[]>([]);
  protected readonly overview = signal<LibraryOverview>({ rootCount: 0, photoCount: 0, videoCount: 0, mediaCount: 0 });
  protected readonly selectedRootId = signal<string | null>(null);
  protected readonly mediaItems = signal<MediaItem[]>([]);
  protected readonly selectedMediaId = signal<string | null>(null);
  protected readonly isAddingFolder = signal(false);
  protected readonly isLoadingMedia = signal(false);
  protected readonly sidebarVisible = signal(true);
  protected readonly inspectorVisible = signal(true);
  protected readonly zoom = signal(100);
  protected readonly themeMode = signal<ThemeMode>("auto");
  protected readonly prefersDark = signal(false);
  protected readonly lastScan = signal<ScanStats | null>(null);
  protected readonly errorMessage = signal<string | null>(null);

  private readonly persistTheme = effect(() => {
    localStorage.setItem("moments.theme", this.themeMode());
  });

  protected readonly selectedRoot = computed(() => {
    const selectedRootId = this.selectedRootId();
    return this.roots().find((root) => root.id === selectedRootId) ?? this.roots()[0] ?? null;
  });

  protected readonly selectedRootName = computed(() => this.selectedRoot()?.name ?? "No folder selected");
  protected readonly selectedRootPath = computed(() => this.selectedRoot()?.path ?? "-");
  protected readonly selectedRootStatus = computed(() => this.selectedRoot()?.status ?? "ready");
  protected readonly selectedRootPhotoCount = computed(() => this.selectedRoot()?.photoCount ?? 0);
  protected readonly selectedRootVideoCount = computed(() => this.selectedRoot()?.videoCount ?? 0);
  protected readonly selectedRootMediaCount = computed(() => this.selectedRoot()?.mediaCount ?? 0);
  protected readonly selectedMedia = computed(() => this.mediaItems().find((item) => item.id === this.selectedMediaId()) ?? this.mediaItems()[0] ?? null);
  protected readonly selectedMediaName = computed(() => this.selectedMedia()?.name ?? "");
  protected readonly selectedMediaPath = computed(() => this.selectedMedia()?.path ?? "");
  protected readonly selectedMediaIndex = computed(() => {
    const selected = this.selectedMedia();
    return selected ? this.mediaItems().findIndex((item) => item.id === selected.id) : -1;
  });
  protected readonly effectiveTheme = computed(() => this.themeMode() === "auto" ? (this.prefersDark() ? "dark" : "light") : this.themeMode());
  protected readonly zoomedImageWidth = computed(() => this.zoom() > 100 ? `${this.zoom()}%` : "auto");
  protected readonly zoomedImageMaxWidth = computed(() => this.zoom() > 100 ? "none" : "100%");
  protected readonly zoomedImageMaxHeight = computed(() => this.zoom() > 100 ? "none" : "100%");
  protected readonly totalMedia = computed(() => this.overview().mediaCount.toLocaleString());

  async ngOnInit(): Promise<void> {
    const savedTheme = localStorage.getItem("moments.theme");
    if (savedTheme === "auto" || savedTheme === "light" || savedTheme === "dark") {
      this.themeMode.set(savedTheme);
    }

    const mediaQuery = window.matchMedia("(prefers-color-scheme: dark)");
    this.prefersDark.set(mediaQuery.matches);
    mediaQuery.addEventListener("change", (event) => this.prefersDark.set(event.matches));

    await this.refreshLibrary();
  }

  @HostListener("document:keydown", ["$event"])
  protected handleKeyboard(event: KeyboardEvent): void {
    if (this.isTextInput(event.target)) {
      return;
    }

    if (event.key === "ArrowLeft") {
      event.preventDefault();
      this.previousMedia();
      return;
    }

    if (event.key === "ArrowRight") {
      event.preventDefault();
      this.nextMedia();
      return;
    }

    if (event.key === "+" || event.key === "=") {
      event.preventDefault();
      this.zoomIn();
      return;
    }

    if (event.key === "-" || event.key === "_") {
      event.preventDefault();
      this.zoomOut();
      return;
    }

    if (event.key === "0") {
      event.preventDefault();
      this.resetZoom();
    }
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
      await this.scanRoot(root);
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
      await this.loadMedia(root.id);
    } catch (error) {
      this.errorMessage.set(this.describeError(error));
      await this.refreshLibrary();
    }
  }

  protected async selectRoot(root: LibraryRoot): Promise<void> {
    this.selectedRootId.set(root.id);
    this.selectedMediaId.set(null);
    await this.loadMedia(root.id);
  }

  protected selectMedia(item: MediaItem): void {
    this.selectedMediaId.set(item.id);
    this.zoom.set(100);
  }

  protected async openNative(item: MediaItem | null = this.selectedMedia()): Promise<void> {
    if (!item) {
      return;
    }

    try {
      await openPath(item.path);
    } catch (error) {
      this.errorMessage.set(this.describeError(error));
    }
  }

  protected previousMedia(): void {
    const index = this.selectedMediaIndex();
    if (index > 0) {
      this.selectMedia(this.mediaItems()[index - 1]);
    }
  }

  protected nextMedia(): void {
    const index = this.selectedMediaIndex();
    const next = this.mediaItems()[index + 1];
    if (next) {
      this.selectMedia(next);
    }
  }

  protected zoomIn(): void {
    this.zoom.update((value) => Math.min(400, value + 25));
  }

  protected zoomOut(): void {
    this.zoom.update((value) => Math.max(25, value - 25));
  }

  protected resetZoom(): void {
    this.zoom.set(100);
  }

  protected closeInspector(): void {
    this.inspectorVisible.set(false);
  }

  protected setThemeMode(mode: ThemeMode): void {
    this.themeMode.set(mode);
  }

  protected toggleChrome(): void {
    const shouldShow = !(this.sidebarVisible() || this.inspectorVisible());
    this.sidebarVisible.set(shouldShow);
    this.inspectorVisible.set(shouldShow);
  }

  protected isSelected(root: LibraryRoot): boolean {
    return this.selectedRoot()?.id === root.id;
  }

  protected isSelectedMedia(item: MediaItem): boolean {
    return this.selectedMedia()?.id === item.id;
  }

  protected mediaUrl(item: MediaItem): string {
    return convertFileSrc(item.path);
  }

  private async refreshLibrary(): Promise<void> {
    const [roots, overview] = await Promise.all([
      invoke<LibraryRoot[]>("list_library_roots"),
      invoke<LibraryOverview>("library_overview"),
    ]);

    this.roots.set(roots);
    this.overview.set(overview);
  }

  private async loadMedia(rootId: string): Promise<void> {
    this.isLoadingMedia.set(true);
    try {
      const media = await invoke<MediaItem[]>("get_library_media", { rootId, offset: 0, limit: 500 });
      this.mediaItems.set(media);
      if (!media.some((item) => item.id === this.selectedMediaId())) {
        this.selectedMediaId.set(media[0]?.id ?? null);
      }
    } catch (error) {
      this.errorMessage.set(this.describeError(error));
    } finally {
      this.isLoadingMedia.set(false);
    }
  }

  private describeError(error: unknown): string {
    return error instanceof Error ? error.message : String(error);
  }

  private isTextInput(target: EventTarget | null): boolean {
    return target instanceof HTMLInputElement || target instanceof HTMLTextAreaElement || target instanceof HTMLSelectElement;
  }
}
