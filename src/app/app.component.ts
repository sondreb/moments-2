import { CommonModule } from "@angular/common";
import { Component, ElementRef, HostListener, OnInit, computed, effect, signal, viewChild } from "@angular/core";
import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { AiModelInfo, DatabaseStats, SettingsComponent } from "./settings.component";

type LibraryRootStatus = "ready" | "scanning" | "error";
type MediaType = "photo" | "video";
type ThemeMode = "auto" | "light" | "dark";
type LibraryView = "library" | "favorites" | "people" | "duplicates" | "recents" | "imports";
type ThumbnailLayoutMode = "dynamic" | "square" | "full";

interface SampleManifestEntry {
  name: string;
  mediaType?: MediaType;
}

const WEB_SAMPLES_ROOT_ID = "web-samples";
const SAMPLE_PHOTO_EXTENSIONS = new Set(["jpg", "jpeg", "png", "webp", "gif", "bmp", "avif"]);
const SAMPLE_VIDEO_EXTENSIONS = new Set(["mp4", "webm", "mov", "m4v"]);

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
  contentHash: string | null;
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

interface FolderOperationResult {
  rootId: string;
  affectedMedia: number;
  message: string;
}

interface MediaDeleteResult {
  deletedMedia: number;
  failedPaths: string[];
  message: string;
}

interface DuplicateGroup {
  hash: string;
  items: MediaItem[];
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

interface Size {
  width: number;
  height: number;
}

interface MediaContextMenuState {
  item: MediaItem;
  x: number;
  y: number;
}

@Component({
  selector: "app-root",
  imports: [CommonModule, SettingsComponent],
  templateUrl: "./app.component.html",
  styleUrl: "./app.component.css",
})
export class AppComponent implements OnInit {
  private static readonly VIEWER_CHROME_IDLE_MS = 1800;
  private static readonly VIEWER_TOP_ZONE_PX = 180;
  private static readonly VIEWER_BOTTOM_ZONE_PX = 180;

  protected readonly desktopAvailable = this.isDesktopRuntime();
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
  protected readonly viewerSurfaceSize = signal<Size | null>(null);
  protected readonly selectedImageNaturalSize = signal<Size | null>(null);
  protected readonly detailToolbarVisible = signal(false);
  protected readonly detailCarouselVisible = signal(false);
  protected readonly thumbnailSize = signal(136);
  protected readonly thumbnailLayoutMode = signal<ThumbnailLayoutMode>("dynamic");
  protected readonly themeMode = signal<ThemeMode>("auto");
  protected readonly prefersDark = signal(false);
  protected readonly lastScan = signal<ScanStats | null>(null);
  protected readonly errorMessage = signal<string | null>(null);
  protected readonly portraitMediaIds = signal<Set<string>>(new Set());
  protected readonly openRootMenuId = signal<string | null>(null);
  protected readonly duplicateGroups = signal<DuplicateGroup[]>([]);
  protected readonly duplicateFilterRootId = signal<string | null>(null);
  protected readonly duplicateDeleteSelection = signal<Set<string>>(new Set());
  protected readonly mediaContextMenu = signal<MediaContextMenuState | null>(null);
  private readonly viewerSurfaceRef = viewChild<ElementRef<HTMLElement>>("viewerSurface");
  private webSampleMedia: MediaItem[] = [];
  private detailToolbarTimer: ReturnType<typeof setTimeout> | null = null;
  private detailCarouselTimer: ReturnType<typeof setTimeout> | null = null;

  private readonly persistTheme = effect(() => {
    localStorage.setItem("moments.theme", this.themeMode());
  });

  private readonly persistSelectedRoot = effect(() => {
    const selectedRootId = this.selectedRootId();
    if (selectedRootId) {
      localStorage.setItem("moments.selectedRootId", selectedRootId);
    }
  });

  private readonly persistThumbnailSize = effect(() => {
    localStorage.setItem("moments.thumbnailSize", String(this.thumbnailSize()));
  });

  private readonly watchViewerSurface = effect((onCleanup) => {
    const surface = this.viewerSurfaceRef()?.nativeElement;
    if (!surface) {
      this.viewerSurfaceSize.set(null);
      return;
    }

    const updateSurfaceSize = () => this.viewerSurfaceSize.set({ width: surface.clientWidth, height: surface.clientHeight });
    updateSurfaceSize();

    const observer = new ResizeObserver(() => updateSurfaceSize());
    observer.observe(surface);
    onCleanup(() => observer.disconnect());
  });

  private readonly persistThumbnailLayoutMode = effect(() => {
    localStorage.setItem("moments.thumbnailLayoutMode", this.thumbnailLayoutMode());
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
  protected readonly detailMediaItems = computed(() => this.activeView() === "favorites" ? this.visibleMediaItems() : this.mediaItems());
  protected readonly selectedMediaMetadata = computed(() => {
    const item = this.selectedMedia();
    return item ? this.metadataById()[item.id] ?? this.emptyMetadata(item.id) : null;
  });
  protected readonly selectedVisibleTags = computed(() => {
    const tags = this.selectedMediaMetadata()?.tags ?? [];
    return tags.filter((tag) => this.shouldDisplayMetadataTag(tag));
  });
  protected readonly selectedHiddenAutoTagCount = computed(() => {
    const tags = this.selectedMediaMetadata()?.tags ?? [];
    return tags.filter((tag) => !this.shouldDisplayMetadataTag(tag)).length;
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
      case "duplicates":
        return this.duplicateFilterRootId() ? `Duplicates in ${this.rootName(this.duplicateFilterRootId())}` : "Duplicates";
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
    return selected ? this.detailMediaItems().findIndex((item) => item.id === selected.id) : -1;
  });
  protected readonly effectiveTheme = computed(() => this.themeMode() === "auto" ? (this.prefersDark() ? "dark" : "light") : this.themeMode());
  protected readonly renderedImageSize = computed<Size | null>(() => {
    const surface = this.viewerSurfaceSize();
    const natural = this.selectedImageNaturalSize();
    const item = this.selectedMedia();
    if (!surface || !natural || !item || item.mediaType !== "photo" || natural.width <= 0 || natural.height <= 0) {
      return null;
    }

    const scale = Math.min(surface.width / natural.width, surface.height / natural.height);
    return {
      width: natural.width * scale,
      height: natural.height * scale,
    };
  });
  protected readonly viewerFrameStyle = computed(() => {
    const size = this.renderedImageSize();
    return {
      transform: `translate3d(${this.panX()}px, ${this.panY()}px, 0) scale(${this.zoom() / 100})`,
      width: size ? `${size.width}px` : "auto",
      height: size ? `${size.height}px` : "auto",
    };
  });
  protected readonly selectedImageStyle = computed(() => {
    const size = this.renderedImageSize();
    return {
      width: size ? `${size.width}px` : "auto",
      height: size ? `${size.height}px` : "auto",
      maxWidth: "none",
      maxHeight: "none",
    };
  });
  protected readonly totalMedia = computed(() => this.overview().mediaCount.toLocaleString());
  protected readonly isScanningFaces = computed(() => this.currentAnalysisTask() === "faces");
  protected readonly isScanningFeatures = computed(() => this.currentAnalysisTask() === "features");
  protected readonly duplicateGroupCount = computed(() => this.duplicateGroups().length);
  private panStart: { pointerId: number; x: number; y: number; panX: number; panY: number } | null = null;

  async ngOnInit(): Promise<void> {
    const savedTheme = localStorage.getItem("moments.theme");
    if (savedTheme === "auto" || savedTheme === "light" || savedTheme === "dark") {
      this.themeMode.set(savedTheme);
    }
    const savedThumbnailSize = Number(localStorage.getItem("moments.thumbnailSize"));
    if (Number.isFinite(savedThumbnailSize) && savedThumbnailSize >= 32 && savedThumbnailSize <= 512) {
      this.thumbnailSize.set(savedThumbnailSize);
    }
    const savedThumbnailLayoutMode = localStorage.getItem("moments.thumbnailLayoutMode");
    if (savedThumbnailLayoutMode === "dynamic" || savedThumbnailLayoutMode === "square" || savedThumbnailLayoutMode === "full") {
      this.thumbnailLayoutMode.set(savedThumbnailLayoutMode);
    }

    const mediaQuery = window.matchMedia("(prefers-color-scheme: dark)");
    this.prefersDark.set(mediaQuery.matches);
    mediaQuery.addEventListener("change", (event) => this.prefersDark.set(event.matches));

    await this.refreshLibrary();
    if (this.desktopAvailable) {
      await this.refreshSettings();
    } else {
      this.settingsMessage.set("Desktop features are unavailable in the browser preview.");
    }
    const savedRootId = localStorage.getItem("moments.selectedRootId");
    const initialRoot = this.roots().find((root) => root.id === savedRootId) ?? this.roots()[0] ?? null;
    if (initialRoot) {
      this.selectedRootId.set(initialRoot.id);
      await this.loadMedia(initialRoot.id);
    }
  }

  @HostListener("document:click")
  protected handleDocumentClick(): void {
    this.openRootMenuId.set(null);
    this.mediaContextMenu.set(null);
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
      return;
    }

    if (event.key === "Escape") {
      this.mediaContextMenu.set(null);
    }
  }

  protected async addFolder(): Promise<void> {
    if (!this.requireDesktopFeature("Adding folders")) {
      return;
    }

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
    if (view === "duplicates" && !this.requireDesktopFeature("Duplicate management")) {
      return;
    }

    this.activeView.set(view);
    this.isDetailOpen.set(false);
    this.isSettingsOpen.set(false);
    if (view === "duplicates") {
      this.duplicateFilterRootId.set(null);
      void this.loadDuplicateGroups();
    }
  }

  protected async refreshSettings(): Promise<void> {
    if (!this.desktopAvailable) {
      this.aiModels.set([]);
      this.databaseStats.set(null);
      return;
    }

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
    if (!this.requireDesktopFeature("Scanning folders")) {
      return;
    }

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
      if (this.activeView() === "duplicates") {
        await this.loadDuplicateGroups();
      }
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
    this.openRootMenuId.set(null);
    await this.loadMedia(root.id);
  }

  protected async openDuplicateItem(item: MediaItem): Promise<void> {
    const root = this.roots().find((candidate) => candidate.id === item.rootId);
    if (!root) {
      return;
    }

    if (this.selectedRootId() !== root.id) {
      this.selectedRootId.set(root.id);
      await this.loadMedia(root.id);
    }

    this.selectedMediaId.set(item.id);
    this.isDetailOpen.set(true);
    this.isSettingsOpen.set(false);
    this.zoom.set(100);
    this.resetPan();
    this.revealDetailToolbar();
    this.hideDetailCarousel();
    this.queueCarouselSync();
  }

  protected selectPerson(person: PersonResult): void {
    if (!person.media) {
      return;
    }

    this.selectMedia(person.media);
  }

  protected selectMedia(item: MediaItem): void {
    this.selectedMediaId.set(item.id);
    this.selectedImageNaturalSize.set(null);
    this.isDetailOpen.set(true);
    this.isSettingsOpen.set(false);
    this.zoom.set(100);
    this.resetPan();
    this.tagDraft.set("");
    this.revealDetailToolbar();
    this.queueCarouselSync();
  }

  protected handleDetailPointerMove(event: PointerEvent): void {
    const viewerSurface = event.currentTarget;
    if (!(viewerSurface instanceof HTMLElement)) {
      return;
    }

    const rect = viewerSurface.getBoundingClientRect();
    const offsetTop = event.clientY - rect.top;
    const offsetBottom = rect.bottom - event.clientY;

    if (offsetTop <= AppComponent.VIEWER_TOP_ZONE_PX) {
      this.revealDetailToolbar();
    }

    if (offsetBottom <= AppComponent.VIEWER_BOTTOM_ZONE_PX && this.detailMediaItems().length > 1) {
      this.revealDetailCarousel();
    }
  }

  protected hideDetailOverlays(): void {
    this.hideDetailToolbar();
    this.hideDetailCarousel();
  }

  protected holdDetailToolbar(): void {
    this.clearDetailToolbarTimer();
    if (this.isDetailOpen()) {
      this.detailToolbarVisible.set(true);
    }
  }

  protected scheduleDetailToolbarHide(): void {
    this.revealDetailToolbar(false);
  }

  protected holdDetailCarousel(): void {
    this.clearDetailCarouselTimer();
    if (this.isDetailOpen() && this.detailMediaItems().length > 1) {
      this.detailCarouselVisible.set(true);
    }
  }

  protected scheduleDetailCarouselHide(): void {
    this.revealDetailCarousel(false);
  }

  protected async openNative(item: MediaItem | null = this.selectedMedia()): Promise<void> {
    if (!item) {
      return;
    }

    if (!this.requireDesktopFeature("Opening media in the native shell")) {
      return;
    }

    try {
      await invoke("open_media_path", { mediaId: item.id });
    } catch (error) {
      this.errorMessage.set(this.describeError(error));
    }
  }

  protected dismissErrorMessage(): void {
    this.errorMessage.set(null);
  }

  protected dismissAnalysisMessage(): void {
    this.analysisMessage.set("");
  }

  protected closeDetail(): void {
    this.isDetailOpen.set(false);
    this.selectedImageNaturalSize.set(null);
    this.zoom.set(100);
    this.resetPan();
    this.mediaContextMenu.set(null);
    this.hideDetailOverlays();
  }

  protected async toggleFavorite(item: MediaItem | null = this.selectedMedia()): Promise<void> {
    if (!item) {
      return;
    }

    if (!this.requireDesktopFeature("Favorites")) {
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

    if (!this.requireDesktopFeature("Tag editing")) {
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

    if (!this.requireDesktopFeature("Tag editing")) {
      return;
    }

    await this.saveTags(item.id, metadata.tags.filter((candidate) => candidate !== tag));
  }

  protected async analyzeFaces(): Promise<void> {
    const item = this.selectedMedia();
    if (!item || item.mediaType !== "photo") {
      return;
    }

    if (!this.requireDesktopFeature("Face analysis")) {
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
    if (!this.requireDesktopFeature("Face naming")) {
      return;
    }

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
      this.selectMedia(this.detailMediaItems()[index - 1]);
      this.isDetailOpen.set(true);
    }
  }

  protected nextMedia(): void {
    const index = this.selectedMediaIndex();
    const next = this.detailMediaItems()[index + 1];
    if (next) {
      this.selectMedia(next);
      this.isDetailOpen.set(true);
    }
  }

  protected zoomIn(): void {
    this.revealDetailToolbar();
    this.zoom.update((value) => Math.min(400, value + 25));
  }

  protected zoomOut(): void {
    this.revealDetailToolbar();
    this.zoom.update((value) => {
      const next = Math.max(25, value - 25);
      if (next <= 100) {
        this.resetPan();
      }
      return next;
    });
  }

  protected resetZoom(): void {
    this.revealDetailToolbar();
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

  protected handleSelectedImageLoad(event: Event): void {
    const image = event.currentTarget;
    if (!(image instanceof HTMLImageElement)) {
      return;
    }

    this.selectedImageNaturalSize.set({ width: image.naturalWidth, height: image.naturalHeight });
  }

  protected setThumbnailSize(value: string): void {
    const next = Number(value);
    if (Number.isFinite(next)) {
      this.thumbnailSize.set(Math.min(512, Math.max(32, next)));
    }
  }

  protected setThumbnailLayoutMode(mode: ThumbnailLayoutMode): void {
    this.thumbnailLayoutMode.set(mode);
  }

  protected openMediaContextMenu(item: MediaItem, event: MouseEvent): void {
    event.preventDefault();
    event.stopPropagation();
    this.mediaContextMenu.set({ item, x: event.clientX, y: event.clientY });
  }

  protected keepMediaContextMenuOpen(event: Event): void {
    event.stopPropagation();
  }

  protected async openInFileExplorer(item: MediaItem | null = this.mediaContextMenu()?.item ?? this.selectedMedia()): Promise<void> {
    if (!item) {
      return;
    }

    if (!this.requireDesktopFeature("Opening media in File Explorer")) {
      return;
    }

    try {
      await invoke("show_media_in_explorer", { mediaId: item.id });
      this.mediaContextMenu.set(null);
    } catch (error) {
      this.errorMessage.set(this.describeError(error));
    }
  }

  protected toggleChrome(): void {
    const shouldShow = !(this.sidebarVisible() || this.inspectorVisible());
    this.sidebarVisible.set(shouldShow);
    this.inspectorVisible.set(shouldShow);
  }

  protected toggleSidebar(): void {
    this.sidebarVisible.update((visible) => !visible);
  }

  protected isSelected(root: LibraryRoot): boolean {
    return this.selectedRoot()?.id === root.id;
  }

  protected isSelectedMedia(item: MediaItem): boolean {
    return this.selectedMedia()?.id === item.id;
  }

  protected mediaUrl(item: MediaItem): string {
    return this.desktopAvailable && !this.isBrowserAssetPath(item.path) ? convertFileSrc(item.path) : item.path;
  }

  protected shouldDisplayMetadataTag(tag: string): boolean {
    const normalized = tag.trim().toLowerCase();
    return normalized.length > 0
      && !normalized.startsWith("imagenet-")
      && normalized !== "onnx-classified"
      && normalized !== "photo"
      && normalized !== "low-light"
      && normalized !== "bright";
  }

  protected carouselItemId(item: MediaItem): string {
    return `carousel-item-${item.id}`;
  }

  protected rootName(rootId: string | null): string {
    return this.roots().find((root) => root.id === rootId)?.name ?? "folder";
  }

  protected faceBoxStyle(face: FaceCandidate): Record<string, string> {
    return {
      left: `${face.x * 100}%`,
      top: `${face.y * 100}%`,
      width: `${face.width * 100}%`,
      height: `${face.height * 100}%`,
    };
  }

  protected personThumbnailMedia(person: PersonGroup): MediaItem | null {
    const face = person.faces[0];
    if (!face) {
      return null;
    }

    return this.mediaItems().find((item) => item.id === face.mediaId) ?? null;
  }

  protected personThumbnailImageStyle(face: FaceCandidate): Record<string, string> {
    const scale = Math.max(1, 1 / Math.max(face.width, face.height, 0.01));
    const centerX = face.x + face.width / 2;
    const centerY = face.y + face.height / 2;

    return {
      left: `${50 - centerX * scale * 100}%`,
      top: `${50 - centerY * scale * 100}%`,
      width: `${scale * 100}%`,
      height: `${scale * 100}%`,
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

  protected isDuplicateSelected(mediaId: string): boolean {
    return this.duplicateDeleteSelection().has(mediaId);
  }

  protected toggleDuplicateSelection(mediaId: string, checked: boolean): void {
    this.duplicateDeleteSelection.update((selection) => {
      const next = new Set(selection);
      if (checked) {
        next.add(mediaId);
      } else {
        next.delete(mediaId);
      }
      return next;
    });
  }

  protected toggleRootMenu(rootId: string, event: Event): void {
    event.stopPropagation();
    this.openRootMenuId.update((current) => current === rootId ? null : rootId);
  }

  protected keepRootMenuOpen(event: Event): void {
    event.stopPropagation();
  }

  protected async removeRoot(root: LibraryRoot, event: Event): Promise<void> {
    event.stopPropagation();
    if (!this.requireDesktopFeature("Removing folders")) {
      return;
    }
    if (!window.confirm(`Remove ${root.name} from Moments?`)) {
      return;
    }

    try {
      const result = await invoke<FolderOperationResult>("remove_library_root", { rootId: root.id });
      this.analysisMessage.set(result.message);
      this.openRootMenuId.set(null);
      await this.refreshLibrary();
      if (this.selectedRootId() === root.id) {
        const nextRoot = this.roots()[0] ?? null;
        this.selectedRootId.set(nextRoot?.id ?? null);
        this.mediaItems.set([]);
        if (nextRoot) {
          await this.loadMedia(nextRoot.id);
        }
      }
      if (this.activeView() === "duplicates") {
        await this.loadDuplicateGroups();
      }
    } catch (error) {
      this.errorMessage.set(this.describeError(error));
    }
  }

  protected async renameRootFiles(root: LibraryRoot, event: Event): Promise<void> {
    event.stopPropagation();
    if (!this.requireDesktopFeature("Renaming files")) {
      return;
    }
    if (!window.confirm(`Rename media in ${root.name} to date-based filenames?`)) {
      return;
    }

    try {
      const result = await invoke<FolderOperationResult>("rename_root_media_by_date", { rootId: root.id });
      this.analysisMessage.set(result.message);
      this.openRootMenuId.set(null);
      await this.refreshLibrary();
      if (this.selectedRootId() === root.id) {
        await this.loadMedia(root.id);
      }
      if (this.activeView() === "duplicates") {
        await this.loadDuplicateGroups();
      }
    } catch (error) {
      this.errorMessage.set(this.describeError(error));
    }
  }

  protected async showRootDuplicates(root: LibraryRoot, event: Event): Promise<void> {
    event.stopPropagation();
    this.openRootMenuId.set(null);
    this.activeView.set("duplicates");
    this.isDetailOpen.set(false);
    this.isSettingsOpen.set(false);
    this.duplicateFilterRootId.set(root.id);
    await this.loadDuplicateGroups(root.id);
  }

  protected async deleteSelectedDuplicates(): Promise<void> {
    if (!this.requireDesktopFeature("Deleting duplicates")) {
      return;
    }

    const mediaIds = [...this.duplicateDeleteSelection()];
    if (mediaIds.length === 0) {
      return;
    }
    if (!window.confirm(`Delete ${mediaIds.length} selected duplicate files from disk?`)) {
      return;
    }

    try {
      const result = await invoke<MediaDeleteResult>("delete_media_items", { mediaIds });
      this.analysisMessage.set(result.message);
      if (result.failedPaths.length > 0) {
        this.errorMessage.set(`Failed to delete ${result.failedPaths.length} files.`);
      }
      this.duplicateDeleteSelection.set(new Set());
      await this.refreshLibrary();
      if (this.selectedRootId()) {
        await this.loadMedia(this.selectedRootId()!);
      }
      await this.loadDuplicateGroups(this.duplicateFilterRootId() ?? undefined);
    } catch (error) {
      this.errorMessage.set(this.describeError(error));
    }
  }

  private async refreshLibrary(): Promise<void> {
    if (!this.desktopAvailable) {
      this.webSampleMedia = await this.loadWebSamples();
      const photoCount = this.webSampleMedia.filter((item) => item.mediaType === "photo").length;
      const videoCount = this.webSampleMedia.filter((item) => item.mediaType === "video").length;
      const roots = this.webSampleMedia.length > 0
        ? [{
            id: WEB_SAMPLES_ROOT_ID,
            name: "Samples",
            path: "/samples",
            status: "ready" as const,
            photoCount,
            videoCount,
            mediaCount: this.webSampleMedia.length,
          }]
        : [];
      this.roots.set(roots);
      this.overview.set({
        rootCount: roots.length,
        photoCount,
        videoCount,
        mediaCount: this.webSampleMedia.length,
      });
      if (this.selectedRootId() && !roots.some((root) => root.id === this.selectedRootId())) {
        this.selectedRootId.set(roots[0]?.id ?? null);
      }
      return;
    }

    const [roots, overview] = await Promise.all([
      invoke<LibraryRoot[]>("list_library_roots"),
      invoke<LibraryOverview>("library_overview"),
    ]);

    this.roots.set(roots);
    this.overview.set(overview);
    if (this.selectedRootId() && !roots.some((root) => root.id === this.selectedRootId())) {
      this.selectedRootId.set(roots[0]?.id ?? null);
    }
  }

  private async loadMedia(rootId: string): Promise<void> {
    this.isLoadingMedia.set(true);
    try {
      if (!this.desktopAvailable) {
        const media = rootId === WEB_SAMPLES_ROOT_ID ? this.webSampleMedia : [];
        this.mediaItems.set(media);
        this.metadataById.set({});
        this.selectedMediaId.set(media.find((item) => item.id === this.selectedMediaId())?.id ?? media[0]?.id ?? null);
        this.queueCarouselSync();
        return;
      }

      const media = await invoke<MediaItem[]>("get_library_media", { rootId, offset: 0, limit: 500 });
      this.mediaItems.set(media);
      await this.loadMetadata(media.map((item) => item.id));
      if (!media.some((item) => item.id === this.selectedMediaId())) {
        this.selectedMediaId.set(media[0]?.id ?? null);
      }
      this.queueCarouselSync();
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
    if (!this.desktopAvailable) {
      this.metadataById.set({});
      return;
    }

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
    if (!this.requireDesktopFeature("Folder analysis")) {
      return;
    }

    const root = this.selectedRoot();
    if (!root) {
      return;
    }

    this.currentAnalysisTask.set(command === "analyze_root_faces" ? "faces" : "features");
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
      if (this.activeView() === "duplicates") {
        await this.loadDuplicateGroups(this.duplicateFilterRootId() ?? undefined);
      }
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

  private async loadDuplicateGroups(rootId?: string): Promise<void> {
    if (!this.desktopAvailable) {
      this.duplicateGroups.set([]);
      this.duplicateDeleteSelection.set(new Set());
      return;
    }

    const groups = await invoke<DuplicateGroup[]>("list_duplicate_groups", { rootId });
    this.duplicateGroups.set(groups);
    this.duplicateDeleteSelection.update((selection) => {
      const validIds = new Set(groups.flatMap((group) => group.items.map((item) => item.id)));
      return new Set([...selection].filter((mediaId) => validIds.has(mediaId)));
    });
  }

  private resetPan(): void {
    this.panX.set(0);
    this.panY.set(0);
    this.isPanning.set(false);
    this.panStart = null;
  }

  private queueCarouselSync(): void {
    requestAnimationFrame(() => {
      const selected = this.selectedMedia();
      if (!selected || !this.isDetailOpen()) {
        return;
      }

      document.getElementById(this.carouselItemId(selected))?.scrollIntoView({
        behavior: "smooth",
        block: "nearest",
        inline: "center",
      });
    });
  }

  private revealDetailToolbar(scheduleHide = true): void {
    if (!this.isDetailOpen()) {
      return;
    }

    this.detailToolbarVisible.set(true);
    this.clearDetailToolbarTimer();
    if (!scheduleHide) {
      return;
    }

    this.detailToolbarTimer = setTimeout(() => {
      this.detailToolbarVisible.set(false);
      this.detailToolbarTimer = null;
    }, AppComponent.VIEWER_CHROME_IDLE_MS);
  }

  private hideDetailToolbar(): void {
    this.clearDetailToolbarTimer();
    this.detailToolbarVisible.set(false);
  }

  private clearDetailToolbarTimer(): void {
    if (this.detailToolbarTimer !== null) {
      clearTimeout(this.detailToolbarTimer);
      this.detailToolbarTimer = null;
    }
  }

  private revealDetailCarousel(scheduleHide = true): void {
    if (!this.isDetailOpen() || this.detailMediaItems().length <= 1) {
      return;
    }

    this.detailCarouselVisible.set(true);
    this.clearDetailCarouselTimer();
    if (!scheduleHide) {
      return;
    }

    this.detailCarouselTimer = setTimeout(() => {
      this.detailCarouselVisible.set(false);
      this.detailCarouselTimer = null;
    }, AppComponent.VIEWER_CHROME_IDLE_MS);
  }

  private hideDetailCarousel(): void {
    this.clearDetailCarouselTimer();
    this.detailCarouselVisible.set(false);
  }

  private clearDetailCarouselTimer(): void {
    if (this.detailCarouselTimer !== null) {
      clearTimeout(this.detailCarouselTimer);
      this.detailCarouselTimer = null;
    }
  }

  private isTextInput(target: EventTarget | null): boolean {
    return target instanceof HTMLInputElement || target instanceof HTMLTextAreaElement || target instanceof HTMLSelectElement;
  }

  private async loadWebSamples(): Promise<MediaItem[]> {
    try {
      const response = await fetch("/samples/manifest.json", { cache: "no-store" });
      if (!response.ok) {
        throw new Error(`failed to load samples manifest (${response.status})`);
      }

      const manifest = await response.json() as SampleManifestEntry[];
      return manifest
        .reduce<MediaItem[]>((items, entry) => {
          const extension = entry.name.split(".").pop()?.toLowerCase() ?? "";
          const mediaType = entry.mediaType
            ?? (SAMPLE_VIDEO_EXTENSIONS.has(extension) ? "video" : "photo");
          if (!SAMPLE_PHOTO_EXTENSIONS.has(extension) && !SAMPLE_VIDEO_EXTENSIONS.has(extension)) {
            return items;
          }

          items.push({
            id: `web-sample-${entry.name}`,
            rootId: WEB_SAMPLES_ROOT_ID,
            name: entry.name,
            path: `/samples/${encodeURIComponent(entry.name)}`,
            mediaType,
            contentHash: null,
          });

          return items;
        }, [])
        .sort((first, second) => first.name.localeCompare(second.name));
    } catch (error) {
      this.errorMessage.set(this.describeError(error));
      return [];
    }
  }

  private isDesktopRuntime(): boolean {
    return typeof window !== "undefined" && typeof (window as Window & { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__ !== "undefined";
  }

  private requireDesktopFeature(feature: string): boolean {
    if (this.desktopAvailable) {
      return true;
    }

    this.errorMessage.set(`${feature} is only available in the desktop app.`);
    return false;
  }

  private isBrowserAssetPath(path: string): boolean {
    return path.startsWith("/") || path.startsWith("http://") || path.startsWith("https://");
  }
}
