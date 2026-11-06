export interface SubjectDef {
  name: string;
  keywords: string[];
  enabled: boolean;
}

export interface AppConfig {
  watchFolder: string | null;
  destinationFolder: string | null;
  monitorOnLaunch: boolean;
  monitoringEnabled: boolean;
  subjects: SubjectDef[];
  onboardingComplete: boolean;
}

export interface MonitorStatus {
  active: boolean;
  watchFolder: string | null;
  message: string;
}

export interface MoveRecord {
  id: string;
  source: string;
  destination: string;
  subject: string;
  fileType: string;
  filename: string;
  timestamp: string;
  undone: boolean;
  batchId?: string | null;
}

export interface PreviewItem {
  source: string;
  destination: string;
  subject: string;
  fileType: string;
  filename: string;
  /** Present when re-sorting files already under the destination. */
  fromSubject?: string | null;
  fromFileType?: string | null;
}

export interface TypeRule {
  extensions: string[];
  folder: string;
}

export interface SubjectRule {
  name: string;
  keywords: string[];
  enabled: boolean;
}

export interface PresetInfo {
  id: string;
  label: string;
}

export interface RulesDto {
  typeRules: TypeRule[];
  subjects: SubjectRule[];
  presets: PresetInfo[];
}

export type TabId = "home" | "import" | "rules" | "settings";

export interface ProposedSubject {
  name: string;
  keywords: string[];
  enabled: boolean;
}

export interface OrganizeProgress {
  phase: string;
  current: number;
  total: number;
  filename: string;
}
