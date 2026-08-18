import { useState } from 'react';
import { X, Folder } from 'lucide-react';

interface ImportDialogProps {
  onClose: () => void;
  onImport: (path: string) => void;
}

export default function ImportDialog({ onClose, onImport }: ImportDialogProps) {
  const [path, setPath] = useState('');
  const [isLoading, setIsLoading] = useState(false);

  const handleImport = async () => {
    if (!path.trim()) return;
    setIsLoading(true);
    try {
      await onImport(path.trim());
      onClose();
    } finally {
      setIsLoading(false);
    }
  };

  // In a real Tauri app, we'd use the file dialog API
  const handleBrowse = () => {
    // Placeholder - in real app: open({ directory: true })
    const demoPath = prompt('Enter folder path to import:');
    if (demoPath) setPath(demoPath);
  };

  return (
    <div className="fixed inset-0 bg-black/60 flex items-center justify-center z-50">
      <div className="bg-surface rounded-lg shadow-xl w-[480px] max-w-[90vw] border border-white/10">
        <div className="flex items-center justify-between p-4 border-b border-white/10">
          <h2 className="text-lg font-semibold">Import Music Folder</h2>
          <button onClick={onClose} className="p-1 hover:bg-white/10 rounded transition-colors">
            <X className="w-5 h-5" />
          </button>
        </div>

        <div className="p-4 space-y-4">
          <p className="text-sm text-text-secondary">
            Select a folder containing audio files (.mp3, .wav, .flac, .ogg, .aiff, .m4a)
          </p>

          <div className="flex gap-2">
            <input
              type="text"
              value={path}
              onChange={(e) => setPath(e.target.value)}
              placeholder="C:\\Music\\MyLibrary"
              className="flex-1 bg-surface-light text-text-primary text-sm rounded-md px-3 py-2 outline-none border border-white/5 focus:border-accent-primary/50"
            />
            <button
              onClick={handleBrowse}
              className="px-3 py-2 bg-surface-light border border-white/5 rounded-md hover:bg-white/5 transition-colors"
            >
              <Folder className="w-4 h-4" />
            </button>
          </div>

          <div className="text-xs text-text-secondary">
            Supported formats: MP3, WAV, FLAC, OGG, AIFF, M4A
          </div>
        </div>

        <div className="flex items-center justify-end gap-2 p-4 border-t border-white/10">
          <button
            onClick={onClose}
            className="px-4 py-2 text-sm text-text-secondary hover:text-text-primary transition-colors"
          >
            Cancel
          </button>
          <button
            onClick={handleImport}
            disabled={!path.trim() || isLoading}
            className="px-4 py-2 bg-accent-primary text-white text-sm rounded-md hover:bg-accent-primary/90 disabled:opacity-50 disabled:cursor-not-allowed transition-colors"
          >
            {isLoading ? 'Importing...' : 'Import'}
          </button>
        </div>
      </div>
    </div>
  );
}
