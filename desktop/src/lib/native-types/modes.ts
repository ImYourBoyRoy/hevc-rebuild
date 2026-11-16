// Run modes and the pages of the app shell.

export type YoutubeMode = 'upscale' | 'optimize';

export type YoutubeSdrCodec = 'h264' | 'hevc';

export type YoutubePreset = 'slow' | 'medium' | 'slower';

export type YoutubeShrink = 'auto' | 'off' | 'aggressive';

export type YoutubeQualityCheck = 'standard' | 'off' | 'relaxed' | 'strict';

export type RunMode = 'check' | 'integrity' | 'metadata' | 'test' | 'full' | 'youtube';

export type Page = 'home' | 'library' | 'metadata' | 'youtube' | 'run' | 'settings' | 'reports' | 'help';
