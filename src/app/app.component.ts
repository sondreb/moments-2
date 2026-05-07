import { CommonModule } from "@angular/common";
import { Component, HostListener, OnInit, computed, effect, signal } from "@angular/core";
import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { AiModelInfo, DatabaseStats, SettingsComponent } from "./settings.component";

type LibraryRootStatus = "ready" | "scanning" | "error";
type MediaType = "photo" | "video";
type ThemeMode = "auto" | "light" | "dark";
type LibraryView = "library" | "favorites" | "people" | "recents" | "imports";

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

interface MediaMetadata {
  mediaId: string;
  favorite: boolean;
  tags: string[];
  faceIds: string[];
}

interface FaceCandidate {
  id: string;
  mediaId: string;
  name: string | null;
  confidence: number;
  x: number;
  y: number;
  width: number;
  height: number;
}

interface FaceAnalysisResult {
  mediaId: string;
  status: "ready" | "modelMissing";
  message: string;
  faces: FaceCandidate[];
}

interface FolderAnalysisResult {
  rootId: string;
  task: string;
  modelId: string;
  processedMedia: number;
  status: "ready" | "modelMissing";
  message: string;
  faces: FaceCandidate[];
  metadata: MediaMetadata[];
}

interface LibraryOverview {
  rootCount: number;
  photoCount: number;
  videoCount: number;
  mediaCount: number;
}

interface PersonResult {
  face: FaceCandidate;
  media: MediaItem | null;
}

interface PersonGroup {
  key: string;
  name: string;
  faces: FaceCandidate[];
  media: MediaItem[];
  confidence: number;
}

@Component({
  selector: "app-root",
  imports: [CommonModule, SettingsComponent],
  templateUrl: "./app.component.html",
  styleUrl: "./app.component.css",
})
export class AppComponent implements OnInit {
  protected readonly roots = signal<LibraryRoot[]>([]);
  protected readonly overview = signal<LibraryOverview>({ rootCount: 0, photoCount: 0, videoCount: 0, mediaCount: 0 });
  protected readonly selectedRootId = signal<string | null>(null);
  protected readonly mediaItems = signal<MediaItem[]>([]);
  protected readonly metadataById = signal<Record<string, MediaMetadata>>({});
  protected readonly faceAnalysisById = signal<Record<string, FaceAnalysisResult>>({});
  protected readonly selectedMediaId = signal<string | null>(null);
  protected readonly activeView = signal<LibraryView>("library");
  protected readonly isDetailOpen = signal(false);
  protected readonly isSettingsOpen = signal(false);
  protected readonly isAddingFolder = signal(false);
  protected readonly isLoadingMedia = signal(false);
  protected readonly isAnalyzingFaces = signal(false);
  protected readonly isInstallingModel = signal(false);
  protected readonly isDeletingModel = signal(false);
  protected readonly isClearingCache = signal(false);
  protected readonly isAnalyzingFolder = signal(false);
  protected readonly tagDraft = signal("");
  protected readonly settingsMessage = signal("");
  protected readonly analysisMessage = signal("");
  protected readonly currentAnalysisTask = signal<"faces" | "features" | null>(null);
  protected readonly aiModels = signal<AiModelInfo[]>([]);
  protected readonly databaseStats = signal<DatabaseStats | null>(null);
  protected readonly sidebarVisible = signal(true);
  protected readonly inspectorVisible = signal(true);
  protected readonly zoom = signal(100);
  protected readonly panX = signal(0);
  protected readonly panY = signal(0);
  protected readonly isPanning = signal(false);
  protected readonly themeMode = signal<ThemeMode>("auto");
  protected readonly prefersDark = signal(false);
  protected readonly lastScan = signal<ScanStats | null>(null);
  protected readonly errorMessage = signal<string | null>(null);
  protected readonly portraitMediaIds = signal<Set<string>>(new Set());

  private readonly persistTheme = effect(() => {
    localStorage.setItem("moments.theme", this.themeMode());
  });

  protected readonly selectedRoot = computed(() => {
    const selectedRootId = this.selectedRootId();
    return this.roots().find((root) => root.id === selectedRootId) ?? this.roots()[0] ?? null;
  });

  protected readonly selectedRootName = computed(() => this.selectedRoot()?.name ?? "No folder selected");
  protected readonly selectedRootIdentifier = computed(() => this.selectedRoot()?.id ?? null);
  protected readonly selectedRootPath = computed(() => this.selectedRoot()?.path ?? "-");
  protected readonly selectedRootStatus = computed(() => this.selectedRoot()?.status ?? "ready");
  protected readonly selectedRootPhotoCount = computed(() => this.selectedRoot()?.photoCount ?? 0);
  protected readonly selectedRootVideoCount = computed(() => this.selectedRoot()?.videoCount ?? 0);
  protected readonly selectedRootMediaCount = computed(() => this.selectedRoot()?.mediaCount ?? 0);
  protected readonly selectedMedia = computed(() => this.mediaItems().find((item) => item.id === this.selectedMediaId()) ?? this.mediaItems()[0] ?? null);
  protected readonly selectedMediaMetadata = computed(() => {
    const item = this.selectedMedia();
    return item ? this.metadataById()[item.id] ?? this.emptyMetadata(item.id) : null;
  });
  protected readonly selectedFaceAnalysis = computed(() => {
    const item = this.selectedMedia();
    return item ? this.faceAnalysisById()[item.id] ?? null : null;
  });
  protected readonly favoriteCount = computed(() => Object.values(this.metadataById()).filter((metadata) => metadata.favorite).length);
  protected readonly people = computed<PersonResult[]>(() => Object.values(this.faceAnalysisById()).flatMap((analysis) => analysis.faces.map((face) => ({
    face,
    media: this.mediaItems().find((item) => item.id === face.mediaId) ?? null,
  }))));
  protected readonly personGroups = computed<PersonGroup[]>(() => {
    const groups = new Map<string, PersonGroup>();
    for (const person of this.people()) {
      const key = person.face.name?.trim().toLocaleLowerCase() || person.face.id;
      const name = person.face.name?.trim() || "Unnamed person";
      const group = groups.get(key) ?? { key, name, faces: [], media: [], confidence: 0 };
      group.faces.push(person.face);
      if (person.media && !group.media.some((item) => item.id === person.media?.id)) {
        group.media.push(person.media);
      }
      group.confidence = Math.max(group.confidence, person.face.confidence);
      groups.set(key, group);
    }
    return [...groups.values()].sort((first, second) => second.faces.length - first.faces.length || first.name.localeCompare(second.name));
  });
  protected readonly peopleCount = computed(() => this.personGroups().length);
  protected readonly visibleMediaItems = computed(() => {
    if (this.activeView() === "favorites") {
      return this.mediaItems().filter((item) => this.metadataById()[item.id]?.favorite);
    }

    return this.mediaItems();
  });
  protected readonly activeViewTitle = computed(() => {
    switch (this.activeView()) {
      case "favorites":
        return "Favorites";
      case "people":
        return "People";
      case "recents":
        return "Recents";
      case "imports":
        return "Imports";
      default:
        return this.selectedRootName();
    }
  });
  protected readonly selectedMediaName = computed(() => this.selectedMedia()?.name ?? "");
  protected readonly selectedMediaPath = computed(() => this.selectedMedia()?.path ?? "");
  protected readonly selectedMediaIndex = computed(() => {
    const selected = this.selectedMedia();
    return selected ? this.mediaItems().findIndex((item) => item.id === selected.id) : -1;
  });
  protected readonly effectiveTheme = computed(() => this.themeMode() === "auto" ? (this.prefersDark() ? "dark" : "light") : this.themeMode());
  protected readonly zoomedImageWidth = computed(() => this.zoom() > 100 ? `${this.zoom()}%` : "100%");
  protected readonly zoomedImageHeight = computed(() => this.zoom() > 100 ? "auto" : "100%");
  protected readonly zoomedImageMaxWidth = computed(() => this.zoom() > 100 ? "none" : "100%");
  protected readonly zoomedImageMaxHeight = computed(() => this.zoom() > 100 ? "none" : "100%");
  protected readonly viewerFrameStyle = computed(() => ({
    transform: `translate3d(${this.panX()}px, ${this.panY()}px, 0)`,
  }));
  protected readonly totalMedia = computed(() => this.overview().mediaCount.toLocaleString());
  protected readonly isScanningFaces = computed(() => this.currentAnalysisTask() === "faces");
  protected readonly isScanningFeatures = computed(() => this.currentAnalysisTask() === "features");
  private panStart: { pointerId: number; x: number; y: number; panX: number; panY: number } | null = null;

  async ngOnInit(): Promise<void> {
    const savedTheme = localStorage.getItem("moments.theme");
    if (savedTheme === "auto" || savedTheme === "light" || savedTheme === "dark") {
      this.themeMode.set(savedTheme);
    }

    const mediaQuery = window.matchMedia("(prefers-color-scheme: dark)");
    this.prefersDark.set(mediaQuery.matches);
    mediaQuery.addEventListener("change", (event) => this.prefersDark.set(event.matches));

    await this.refreshLibrary();
    await this.refreshSettings();
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

  protected async openSettings(): Promise<void> {
    this.isDetailOpen.set(false);
    this.isSettingsOpen.set(true);
    await this.refreshSettings();
  }

  protected closeSettings(): void {
    this.isSettingsOpen.set(false);
  }

  protected showView(view: LibraryView): void {
    this.activeView.set(view);
    this.isDetailOpen.set(false);
    this.isSettingsOpen.set(false);
  }

  protected async refreshSettings(): Promise<void> {
    try {
      const [models, databaseStats] = await Promise.all([
        invoke<AiModelInfo[]>("list_ai_models"),
        invoke<DatabaseStats>("get_database_stats"),
      ]);
      this.aiModels.set(models);
      this.databaseStats.set(databaseStats);
    } catch (error) {
      this.errorMessage.set(this.describeError(error));
    }
  }

  protected async installModel(modelId: string): Promise<void> {
    this.isInstallingModel.set(true);
    this.settingsMessage.set("Downloading model...");
    try {
      const result = await invoke<{ model: AiModelInfo; message: string }>("install_ai_model", { modelId });
      this.aiModels.update((models) => models.map((model) => model.id === result.model.id ? result.model : model));
      this.settingsMessage.set(result.message);
    } catch (error) {
      this.settingsMessage.set(this.describeError(error));
    } finally {
      this.isInstallingModel.set(false);
      await this.refreshSettings();
    }
  }

  protected async deleteModel(modelId: string): Promise<void> {
    this.isDeletingModel.set(true);
    try {
      const result = await invoke<{ model: AiModelInfo; removedBytes: number; message: string }>("delete_ai_model", { modelId });
      this.aiModels.update((models) => models.map((model) => model.id === result.model.id ? result.model : model));
      this.settingsMessage.set(`${result.message} Removed ${this.formatBytes(result.removedBytes)}.`);
    } catch (error) {
      this.settingsMessage.set(this.describeError(error));
    } finally {
      this.isDeletingModel.set(false);
      await this.refreshSettings();
    }
  }

  protected async analyzeRootFaces(): Promise<void> {
    await this.runFolderAnalysis("analyze_root_faces");
  }

  protected async classifyRootImages(): Promise<void> {
    await this.runFolderAnalysis("classify_root_images");
  }

  protected async clearCache(): Promise<void> {
    this.isClearingCache.set(true);
    try {
      const result = await invoke<{ removedFiles: number; removedBytes: number }>("clear_app_cache");
      this.settingsMessage.set(`Removed ${result.removedFiles} cache files (${this.formatBytes(result.removedBytes)}).`);
    } catch (error) {
      this.settingsMessage.set(this.describeError(error));
    } finally {
      this.isClearingCache.set(false);
      await this.refreshSettings();
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
    this.isDetailOpen.set(false);
    this.activeView.set("library");
    await this.loadMedia(root.id);
  }

  protected selectPerson(person: PersonResult): void {
    if (!person.media) {
      return;
    }

    this.selectMedia(person.media);
  }

  protected selectMedia(item: MediaItem): void {
    this.selectedMediaId.set(item.id);
    this.isDetailOpen.set(true);
    this.isSettingsOpen.set(false);
    this.zoom.set(100);
    this.resetPan();
    this.tagDraft.set("");
  }

  protected async openNative(item: MediaItem | null = this.selectedMedia()): Promise<void> {
    if (!item) {
      return;
    }

    try {
      await invoke("open_media_path", { mediaId: item.id });
    } catch (error) {
      this.errorMessage.set(this.describeError(error));
    }
  }

  protected closeDetail(): void {
    this.isDetailOpen.set(false);
    this.zoom.set(100);
    this.resetPan();
  }

  protected async toggleFavorite(item: MediaItem | null = this.selectedMedia()): Promise<void> {
    if (!item) {
      return;
    }

    const current = this.metadataById()[item.id] ?? this.emptyMetadata(item.id);
    try {
      const metadata = await invoke<MediaMetadata>("set_media_favorite", {
        mediaId: item.id,
        favorite: !current.favorite,
      });
      this.upsertMetadata(metadata);
    } catch (error) {
      this.errorMessage.set(this.describeError(error));
    }
  }

  protected async addTag(): Promise<void> {
    const item = this.selectedMedia();
    const tag = this.tagDraft().trim();
    if (!item || !tag) {
      return;
    }

    const metadata = this.selectedMediaMetadata() ?? this.emptyMetadata(item.id);
    await this.saveTags(item.id, [...metadata.tags, tag]);
    this.tagDraft.set("");
  }

  protected async removeTag(tag: string): Promise<void> {
    const item = this.selectedMedia();
    const metadata = this.selectedMediaMetadata();
    if (!item || !metadata) {
      return;
    }

    await this.saveTags(item.id, metadata.tags.filter((candidate) => candidate !== tag));
  }

  protected async analyzeFaces(): Promise<void> {
    const item = this.selectedMedia();
    if (!item || item.mediaType !== "photo") {
      return;
    }

    this.isAnalyzingFaces.set(true);
    try {
      const result = await invoke<FaceAnalysisResult>("analyze_media_faces", { mediaId: item.id });
      this.faceAnalysisById.update((analysis) => ({ ...analysis, [item.id]: result }));
    } catch (error) {
      this.errorMessage.set(this.describeError(error));
    } finally {
      this.isAnalyzingFaces.set(false);
    }
  }

  protected async renameFace(face: FaceCandidate, event: Event): Promise<void> {
    const input = event.target instanceof HTMLInputElement ? event.target.value : "";
    try {
      const updated = await invoke<FaceCandidate>("set_face_name", { faceId: face.id, name: input });
      this.faceAnalysisById.update((analysis) => {
        const current = analysis[updated.mediaId];
        if (!current) {
          return analysis;
        }
        return {
          ...analysis,
          [updated.mediaId]: {
            ...current,
            faces: current.faces.map((candidate) => candidate.id === updated.id ? updated : candidate),
          },
        };
      });
    } catch (error) {
      this.errorMessage.set(this.describeError(error));
    }
  }

  protected previousMedia(): void {
    const index = this.selectedMediaIndex();
    if (index > 0) {
      this.selectMedia(this.mediaItems()[index - 1]);
      this.isDetailOpen.set(true);
    }
  }

  protected nextMedia(): void {
    const index = this.selectedMediaIndex();
    const next = this.mediaItems()[index + 1];
    if (next) {
      this.selectMedia(next);
      this.isDetailOpen.set(true);
    }
  }

  protected zoomIn(): void {
    this.zoom.update((value) => Math.min(400, value + 25));
  }

  protected zoomOut(): void {
    this.zoom.update((value) => {
      const next = Math.max(25, value - 25);
      if (next <= 100) {
        this.resetPan();
      }
      return next;
    });
  }

  protected resetZoom(): void {
    this.zoom.set(100);
    this.resetPan();
  }

  protected handleViewerWheel(event: WheelEvent): void {
    const item = this.selectedMedia();
    if (!item || item.mediaType !== "photo") {
      return;
    }

    event.preventDefault();
    const delta = event.deltaY < 0 ? 15 : -15;
    this.zoom.update((value) => {
      const next = Math.min(400, Math.max(25, value + delta));
      if (next <= 100) {
        this.resetPan();
      }
      return next;
    });
  }

  protected startPan(event: PointerEvent): void {
    const item = this.selectedMedia();
    if (!item || item.mediaType !== "photo" || this.zoom() <= 100 || event.button !== 0) {
      return;
    }

    event.preventDefault();
    this.isPanning.set(true);
    this.panStart = { pointerId: event.pointerId, x: event.clientX, y: event.clientY, panX: this.panX(), panY: this.panY() };
    (event.currentTarget as HTMLElement).setPointerCapture(event.pointerId);
  }

  protected movePan(event: PointerEvent): void {
    if (!this.panStart || event.pointerId !== this.panStart.pointerId) {
      return;
    }

    this.panX.set(this.panStart.panX + event.clientX - this.panStart.x);
    this.panY.set(this.panStart.panY + event.clientY - this.panStart.y);
  }

  protected endPan(event: PointerEvent): void {
    if (this.panStart?.pointerId === event.pointerId) {
      this.panStart = null;
      this.isPanning.set(false);
    }
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

  protected faceBoxStyle(face: FaceCandidate): Record<string, string> {
    return {
      left: `${face.x * 100}%`,
      top: `${face.y * 100}%`,
      width: `${face.width * 100}%`,
      height: `${face.height * 100}%`,
    };
  }

  protected markTileOrientation(item: MediaItem, event: Event): void {
    const image = event.target instanceof HTMLImageElement ? event.target : null;
    if (!image) {
      return;
    }

    this.portraitMediaIds.update((ids) => {
      const next = new Set(ids);
      if (image.naturalHeight > image.naturalWidth * 1.12) {
        next.add(item.id);
      } else {
        next.delete(item.id);
      }
      return next;
    });
  }

  protected isPortraitTile(item: MediaItem): boolean {
    return this.portraitMediaIds().has(item.id);
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
      await this.loadMetadata(media.map((item) => item.id));
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

  private formatBytes(bytes: number): string {
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

  private async loadMetadata(mediaIds: string[]): Promise<void> {
    if (mediaIds.length === 0) {
      this.metadataById.set({});
      return;
    }

    const metadata = await invoke<MediaMetadata[]>("get_media_metadata", { mediaIds });
    this.metadataById.set(Object.fromEntries(metadata.map((entry) => [entry.mediaId, entry])));
  }

  private async saveTags(mediaId: string, tags: string[]): Promise<void> {
    try {
      const metadata = await invoke<MediaMetadata>("set_media_tags", { mediaId, tags });
      this.upsertMetadata(metadata);
    } catch (error) {
      this.errorMessage.set(this.describeError(error));
    }
  }

  private async runFolderAnalysis(command: "analyze_root_faces" | "classify_root_images"): Promise<void> {
    const root = this.selectedRoot();
    if (!root) {
      return;
    }

    this.currentAnalysisTask.set(command === "analyze_root_faces" ? "faces" : "features");
    this.analysisMessage.set(
      command === "analyze_root_faces"
        ? "Scanning faces with the local model..."
        : "Scanning image features with the local model...",
    );
    this.isAnalyzingFolder.set(true);
    try {
      const result = await invoke<FolderAnalysisResult>(command, { rootId: root.id });
      for (const metadata of result.metadata) {
        this.upsertMetadata(metadata);
      }
      if (command === "analyze_root_faces") {
        const facesByMediaId = new Map<string, FaceCandidate[]>();
        for (const face of result.faces) {
          facesByMediaId.set(face.mediaId, [...(facesByMediaId.get(face.mediaId) ?? []), face]);
        }
        this.faceAnalysisById.update((analysis) => {
          const next = { ...analysis };
          for (const item of this.mediaItems().filter((candidate) => candidate.rootId === root.id && candidate.mediaType === "photo")) {
            next[item.id] = {
              mediaId: item.id,
              status: result.status,
              message: result.message,
              faces: facesByMediaId.get(item.id) ?? [],
            };
          }
          return next;
        });
      }
      this.settingsMessage.set("");
      this.analysisMessage.set("");
    } catch (error) {
      this.settingsMessage.set(this.describeError(error));
      this.analysisMessage.set(this.describeError(error));
    } finally {
      this.isAnalyzingFolder.set(false);
      this.currentAnalysisTask.set(null);
      await this.refreshSettings();
    }
  }

  private upsertMetadata(metadata: MediaMetadata): void {
    this.metadataById.update((entries) => ({ ...entries, [metadata.mediaId]: metadata }));
  }

  private emptyMetadata(mediaId: string): MediaMetadata {
    return { mediaId, favorite: false, tags: [], faceIds: [] };
  }

  private resetPan(): void {
    this.panX.set(0);
    this.panY.set(0);
    this.isPanning.set(false);
    this.panStart = null;
  }

  private isTextInput(target: EventTarget | null): boolean {
    return target instanceof HTMLInputElement || target instanceof HTMLTextAreaElement || target instanceof HTMLSelectElement;
  }
}
