import { useState } from 'react';
import { Wand2, Loader, Check, X, ChevronDown, ChevronRight } from 'lucide-react';
import { assistRepairMetadata, assistApplyMetadataRepair } from '../../lib/tauri';
import type { MetadataProposal, MetadataRepairBatch } from '../../types';

export default function MetadataRepair() {
  const [scanning, setScanning] = useState(false);
  const [batch, setBatch] = useState<MetadataRepairBatch | null>(null);
  const [applying, setApplying] = useState<number | null>(null);
  const [applied, setApplied] = useState<Set<number>>(new Set());
  const [dismissed, setDismissed] = useState<Set<number>>(new Set());
  const [expanded, setExpanded] = useState<Set<number>>(new Set());

  const handleScan = async () => {
    setScanning(true);
    try {
      const result = await assistRepairMetadata();
      setBatch(result);
    } catch (e) {
      console.error('Metadata repair failed:', e);
    } finally {
      setScanning(false);
    }
  };

  const handleApply = async (proposal: MetadataProposal) => {
    setApplying(proposal.trackId);
    try {
      await assistApplyMetadataRepair(proposal);
      setApplied((prev) => new Set(prev).add(proposal.trackId));
    } catch (e) {
      console.error('Failed to apply repair:', e);
    } finally {
      setApplying(null);
    }
  };

  const handleDismiss = (trackId: number) => {
    setDismissed((prev) => new Set(prev).add(trackId));
  };

  const toggleExpand = (trackId: number) => {
    setExpanded((prev) => {
      const next = new Set(prev);
      if (next.has(trackId)) next.delete(trackId);
      else next.add(trackId);
      return next;
    });
  };

  const visibleProposals = batch?.proposals.filter(
    (p) => !applied.has(p.trackId) && !dismissed.has(p.trackId)
  ) ?? [];

  return (
    <div className="flex flex-col h-full overflow-hidden">
      {/* Header */}
      <div className="p-4 border-b border-white/5 bg-surface/30">
        <h2 className="text-lg font-bold text-text-primary mb-1">
          Metadata Repair
        </h2>
        <p className="text-sm text-text-secondary mb-3">
          Scan your library for tracks with missing artist, title, or genre.
          The LLM parses filenames and infers metadata. You review and approve
          each change before it's applied.
        </p>
        <button
          onClick={handleScan}
          disabled={scanning}
          className={`
            flex items-center gap-2 px-4 py-2 rounded-lg text-sm font-medium transition-colors
            ${scanning
              ? 'bg-surface text-text-secondary cursor-not-allowed'
              : 'bg-accent-primary text-white hover:bg-accent-primary/90'
            }
          `}
        >
          {scanning ? (
            <><Loader className="w-4 h-4 animate-spin" /> Scanning library...</>
          ) : (
            <><Wand2 className="w-4 h-4" /> Scan Library</>
          )}
        </button>
        {batch && (
          <div className="mt-2 text-sm text-text-secondary">
            Scanned {batch.totalScanned} tracks, found {batch.totalProposed} with
            missing metadata. {visibleProposals.length} pending review.
          </div>
        )}
      </div>

      {/* Proposals */}
      {visibleProposals.length > 0 && (
        <div className="flex-1 overflow-y-auto p-4 space-y-2">
          {visibleProposals.map((proposal) => (
            <div
              key={proposal.trackId}
              className="bg-surface rounded-lg border border-white/5 overflow-hidden"
            >
              {/* Header row */}
              <div className="flex items-center gap-3 p-3">
                <button
                  onClick={() => toggleExpand(proposal.trackId)}
                  className="text-text-secondary hover:text-text-primary"
                >
                  {expanded.has(proposal.trackId) ? (
                    <ChevronDown className="w-4 h-4" />
                  ) : (
                    <ChevronRight className="w-4 h-4" />
                  )}
                </button>
                <div className="flex-1 min-w-0">
                  <div className="text-sm text-text-primary truncate">
                    {proposal.proposedArtist && proposal.proposedTitle
                      ? `${proposal.proposedArtist} — ${proposal.proposedTitle}`
                      : proposal.filename}
                  </div>
                  <div className="text-xs text-text-secondary truncate">
                    {proposal.filename}
                  </div>
                </div>
                <div className="flex items-center gap-2">
                  <span className={`text-xs px-2 py-0.5 rounded ${
                    proposal.confidence > 0.8
                      ? 'bg-green-500/20 text-green-400'
                      : proposal.confidence > 0.6
                        ? 'bg-yellow-500/20 text-yellow-400'
                        : 'bg-red-500/20 text-red-400'
                  }`}>
                    {Math.round(proposal.confidence * 100)}%
                  </span>
                  <button
                    onClick={() => handleApply(proposal)}
                    disabled={applying === proposal.trackId}
                    className="p-1.5 rounded text-green-400 hover:bg-green-500/20 transition-colors disabled:opacity-40"
                    title="Apply"
                  >
                    {applying === proposal.trackId ? (
                      <Loader className="w-4 h-4 animate-spin" />
                    ) : (
                      <Check className="w-4 h-4" />
                    )}
                  </button>
                  <button
                    onClick={() => handleDismiss(proposal.trackId)}
                    className="p-1.5 rounded text-text-secondary hover:text-red-400 hover:bg-red-500/20 transition-colors"
                    title="Dismiss"
                  >
                    <X className="w-4 h-4" />
                  </button>
                </div>
              </div>

              {/* Expanded details */}
              {expanded.has(proposal.trackId) && (
                <div className="px-3 pb-3 border-t border-white/5 pt-2">
                  <div className="grid grid-cols-2 gap-3 text-xs">
                    <div>
                      <div className="text-text-secondary uppercase tracking-wide mb-1">
                        Current
                      </div>
                      <div>Artist: {proposal.currentArtist || '(empty)'}</div>
                      <div>Title: {proposal.currentTitle || '(empty)'}</div>
                      <div>Album: {proposal.currentAlbum || '(empty)'}</div>
                      <div>Genre: {proposal.currentGenre || '(empty)'}</div>
                    </div>
                    <div>
                      <div className="text-text-secondary uppercase tracking-wide mb-1">
                        Proposed
                      </div>
                      <div>Artist: <span className="text-accent-primary">{proposal.proposedArtist || '(no change)'}</span></div>
                      <div>Title: <span className="text-accent-primary">{proposal.proposedTitle || '(no change)'}</span></div>
                      <div>Album: <span className="text-accent-primary">{proposal.proposedAlbum || '(no change)'}</span></div>
                      <div>Genre: <span className="text-accent-primary">{proposal.proposedGenre || '(no change)'}</span></div>
                    </div>
                  </div>
                  <div className="mt-2 text-xs text-text-secondary">
                    Source: {proposal.source}
                  </div>
                </div>
              )}
            </div>
          ))}
        </div>
      )}

      {/* Empty state */}
      {batch && visibleProposals.length === 0 && (
        <div className="flex-1 flex items-center justify-center">
          <div className="text-center">
            <Check className="w-12 h-12 text-green-400 mx-auto mb-3" />
            <p className="text-text-secondary">
              All proposals reviewed. {applied.size} applied, {dismissed.size} dismissed.
            </p>
          </div>
        </div>
      )}
    </div>
  );
}
