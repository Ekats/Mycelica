import { useState, useEffect, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Document, Page, pdfjs } from 'react-pdf';
import ReactMarkdown from 'react-markdown';
import { ChevronLeft, FileText, ExternalLink, Users, Calendar, BookOpen, ZoomIn, ZoomOut, RotateCcw } from 'lucide-react';
import { useGraphStore } from '../../stores/graphStore';
import mammoth from 'mammoth';
import 'react-pdf/dist/Page/AnnotationLayer.css';
import 'react-pdf/dist/Page/TextLayer.css';

// Bundle worker locally - CDN approach fails silently in Tauri
import pdfjsWorker from 'pdfjs-dist/build/pdf.worker.min.mjs?url';
pdfjs.GlobalWorkerOptions.workerSrc = pdfjsWorker;

interface PaperAuthor {
  fullName: string;
  orcid?: string;
}

interface PaperMetadata {
  id: number;
  nodeId: string;
  openAireId?: string;
  doi?: string;
  authors?: string;  // JSON string
  publicationDate?: string;
  journal?: string;
  publisher?: string;
  abstract?: string;
  abstractFormatted?: string;  // Markdown with **Section** headers
  abstractSections?: string;   // JSON array of detected sections
  pdfUrl?: string;
  pdfAvailable: boolean;
  subjects?: string;  // JSON string
  accessRight?: string;
  createdAt: number;
}

interface PaperViewerProps {
  nodeId: string;
  metadata: PaperMetadata;
  title: string;
  onBack: () => void;
}

// HiDPI: Render at high scale, display at normal size via CSS transform
const PDF_RENDER_SCALE = 6;  // 6x internal resolution for crisp HiDPI
const DEFAULT_DISPLAY_WIDTH = 850;  // Default display width in CSS pixels
const MIN_DISPLAY_WIDTH = 400;
const MAX_DISPLAY_WIDTH = 1600;
const ZOOM_STEP = 100;

export function PaperViewer({ nodeId, metadata, title, onBack }: PaperViewerProps) {
  const leafInitialView = useGraphStore(state => state.leafInitialView);
  const [viewMode, setViewMode] = useState<'abstract' | 'document'>(
    leafInitialView === 'pdf' && metadata.pdfAvailable ? 'document' : 'abstract'
  );
  const [pdfBlobUrl, setPdfBlobUrl] = useState<string | null>(null);
  const [docFormat, setDocFormat] = useState<string | null>(null);  // 'pdf', 'docx', 'doc'
  const [docxHtml, setDocxHtml] = useState<string | null>(null);  // Rendered DOCX HTML
  const [docLoading, setDocLoading] = useState(false);
  const [docError, setDocError] = useState<string | null>(null);
  const [numPages, setNumPages] = useState<number>(0);
  const [pageNumber, setPageNumber] = useState(1);
  const [pageWidth, setPageWidth] = useState<number | null>(null);  // Track actual rendered width
  const [displayWidth, setDisplayWidth] = useState(DEFAULT_DISPLAY_WIDTH);  // User-adjustable zoom

  // Parse authors from JSON string
  const authors: PaperAuthor[] = metadata.authors
    ? JSON.parse(metadata.authors)
    : [];

  // Load document when switching to document view
  useEffect(() => {
    if (viewMode === 'document' && metadata.pdfAvailable && !pdfBlobUrl && !docxHtml) {
      loadDocument();
    }
  }, [viewMode, metadata.pdfAvailable]);

  // Cleanup blob URL on unmount
  useEffect(() => {
    return () => {
      if (pdfBlobUrl) {
        URL.revokeObjectURL(pdfBlobUrl);
      }
    };
  }, [pdfBlobUrl]);

  const loadDocument = async () => {
    setDocLoading(true);
    setDocError(null);
    try {
      const result = await invoke<[number[], string] | null>('get_paper_document', { nodeId });
      if (result) {
        const [data, format] = result;
        setDocFormat(format);

        if (format === 'pdf') {
          const blob = new Blob([new Uint8Array(data)], { type: 'application/pdf' });
          const url = URL.createObjectURL(blob);
          setPdfBlobUrl(url);
        } else if (format === 'docx') {
          // Convert DOCX to HTML using mammoth
          const arrayBuffer = new Uint8Array(data).buffer;
          const { value: html } = await mammoth.convertToHtml({ arrayBuffer });
          setDocxHtml(html);
        } else if (format === 'doc') {
          setDocError('DOC format requires external viewer. Click "Open External" to view.');
        }
      } else {
        setDocError('Document not available');
      }
    } catch (err) {
      console.error('Failed to load document:', err);
      setDocError(err instanceof Error ? err.message : 'Failed to load document');
    } finally {
      setDocLoading(false);
    }
  };

  const onDocumentLoadSuccess = ({ numPages }: { numPages: number }) => {
    setNumPages(numPages);
    setPageNumber(1);
  };

  // Callback when main page renders - capture actual width for CSS scaling
  const onPageRenderSuccess = (page: { width: number }) => {
    setPageWidth(page.width);
  };

  // Calculate CSS scale factor: rendered at PDF_RENDER_SCALE, display at displayWidth
  const cssScale = pageWidth ? displayWidth / pageWidth : 1 / PDF_RENDER_SCALE;

  // Zoom controls
  const zoomIn = () => setDisplayWidth(w => Math.min(w + ZOOM_STEP, MAX_DISPLAY_WIDTH));
  const zoomOut = () => setDisplayWidth(w => Math.max(w - ZOOM_STEP, MIN_DISPLAY_WIDTH));
  const resetZoom = () => setDisplayWidth(DEFAULT_DISPLAY_WIDTH);
  const zoomPercent = Math.round((displayWidth / DEFAULT_DISPLAY_WIDTH) * 100);

  // Open PDF in system viewer (uses Rust command with xdg-open)
  const openInExternalViewer = async () => {
    try {
      await invoke('open_paper_external', { nodeId, title });
    } catch (err) {
      console.error('[PDF] Failed to open in external viewer:', err);
    }
  };

  // Escape key handling: document view → abstract, abstract → back to graph
  const handleEscape = useCallback((e: KeyboardEvent) => {
    if (e.key === 'Escape') {
      if (viewMode === 'document') {
        setViewMode('abstract');
      } else {
        onBack();
      }
    }
  }, [viewMode, onBack]);

  useEffect(() => {
    document.addEventListener('keydown', handleEscape);
    return () => document.removeEventListener('keydown', handleEscape);
  }, [handleEscape]);

  const formatDate = (dateStr?: string) => {
    if (!dateStr) return null;
    try {
      return new Date(dateStr).toLocaleDateString('en-US', {
        year: 'numeric',
        month: 'long',
        day: 'numeric'
      });
    } catch {
      return dateStr;
    }
  };

  return (
    <div className="h-full flex flex-col bg-gray-900 text-white">
      {/* Header */}
      <div className="flex-none border-b border-gray-700 p-4">
        <div className="flex items-center gap-3 mb-3">
          <button
            onClick={onBack}
            className="p-2 hover:bg-gray-700 rounded-lg transition-colors"
            title="Back to graph"
          >
            <ChevronLeft size={20} />
          </button>
          <span className="text-2xl">📄</span>
          <h1 className="text-xl font-semibold flex-1 line-clamp-2">{title}</h1>
        </div>

        {/* Metadata row */}
        <div className="flex flex-wrap gap-4 text-sm text-gray-400">
          {authors.length > 0 && (
            <div className="flex items-center gap-1">
              <Users size={14} />
              <span>{authors.slice(0, 3).map(a => a.fullName).join(', ')}</span>
              {authors.length > 3 && <span className="text-gray-500">+{authors.length - 3} more</span>}
            </div>
          )}
          {metadata.publicationDate && (
            <div className="flex items-center gap-1">
              <Calendar size={14} />
              <span>{formatDate(metadata.publicationDate)}</span>
            </div>
          )}
          {metadata.journal && (
            <div className="flex items-center gap-1">
              <BookOpen size={14} />
              <span>{metadata.journal}</span>
            </div>
          )}
          {metadata.doi && (
            <a
              href={`https://doi.org/${metadata.doi}`}
              target="_blank"
              rel="noopener noreferrer"
              className="flex items-center gap-1 text-amber-400 hover:text-amber-300"
            >
              <ExternalLink size={14} />
              <span>DOI: {metadata.doi}</span>
            </a>
          )}
        </div>

        {/* View mode toggle */}
        <div className="flex gap-2 mt-4">
          <button
            onClick={() => setViewMode('abstract')}
            className={`px-4 py-2 rounded-lg text-sm font-medium transition-colors ${
              viewMode === 'abstract'
                ? 'bg-amber-600 text-white'
                : 'bg-gray-700 text-gray-300 hover:bg-gray-600'
            }`}
          >
            <FileText size={16} className="inline mr-2" />
            Abstract
          </button>
          {metadata.pdfAvailable ? (
            <>
              <button
                onClick={() => setViewMode('document')}
                className={`px-4 py-2 rounded-lg text-sm font-medium transition-colors ${
                  viewMode === 'document'
                    ? 'bg-amber-600 text-white'
                    : 'bg-gray-700 text-gray-300 hover:bg-gray-600'
                }`}
              >
                {docFormat === 'docx' || docFormat === 'doc' ? 'Document' : 'PDF Viewer'}
              </button>
              <button
                onClick={openInExternalViewer}
                className="px-4 py-2 rounded-lg text-sm font-medium bg-gray-700 text-gray-300 hover:bg-gray-600 flex items-center gap-2"
                title="Open in system viewer"
              >
                <ExternalLink size={16} />
                Open External
              </button>
            </>
          ) : metadata.pdfUrl ? (
            <a
              href={metadata.pdfUrl}
              target="_blank"
              rel="noopener noreferrer"
              className="px-4 py-2 rounded-lg text-sm font-medium bg-gray-700 text-gray-300 hover:bg-gray-600 flex items-center gap-2"
            >
              <ExternalLink size={16} />
              Open Paper
            </a>
          ) : null}
        </div>
      </div>

      {/* Content area */}
      <div className="flex-1 overflow-auto p-6">
        {viewMode === 'abstract' ? (
          <div className="max-w-3xl mx-auto">
            <h2 className="text-lg font-semibold mb-4 text-gray-300">Abstract</h2>
            {metadata.abstractFormatted ? (
              <div className="prose prose-invert prose-sm max-w-none text-gray-200 leading-relaxed [&_strong]:text-amber-400 [&_strong]:font-semibold [&_strong]:block [&_strong]:mt-4 [&_strong]:mb-2 [&_strong:first-child]:mt-0">
                <ReactMarkdown>{metadata.abstractFormatted}</ReactMarkdown>
              </div>
            ) : (
              <p className="text-gray-200 leading-relaxed whitespace-pre-wrap">
                {metadata.abstract || 'No abstract available.'}
              </p>
            )}

            {/* Authors list */}
            {authors.length > 0 && (
              <div className="mt-8">
                <h3 className="text-md font-semibold mb-3 text-gray-300">Authors</h3>
                <div className="flex flex-wrap gap-2">
                  {authors.map((author, i) => (
                    <span
                      key={i}
                      className="px-3 py-1 bg-gray-800 rounded-full text-sm text-gray-300"
                    >
                      {author.fullName}
                      {author.orcid && (
                        <a
                          href={`https://orcid.org/${author.orcid}`}
                          target="_blank"
                          rel="noopener noreferrer"
                          className="ml-2 text-green-400 hover:text-green-300"
                        >
                          ORCID
                        </a>
                      )}
                    </span>
                  ))}
                </div>
              </div>
            )}

            {/* Publisher info */}
            {metadata.publisher && (
              <div className="mt-6 text-sm text-gray-400">
                Published by: {metadata.publisher}
              </div>
            )}
          </div>
        ) : (
          <div className="flex flex-col items-center">
            {docLoading ? (
              <div className="text-gray-400">Loading document...</div>
            ) : docError ? (
              <div className="text-red-400">{docError}</div>
            ) : docxHtml ? (
              /* DOCX viewer */
              <div className="max-w-4xl w-full bg-white text-black p-8 rounded-lg shadow-lg">
                <div
                  className="prose prose-sm max-w-none [&_p]:mb-4 [&_h1]:text-2xl [&_h1]:font-bold [&_h2]:text-xl [&_h2]:font-semibold [&_h3]:text-lg [&_h3]:font-medium [&_table]:border-collapse [&_td]:border [&_td]:border-gray-300 [&_td]:p-2 [&_th]:border [&_th]:border-gray-300 [&_th]:p-2 [&_th]:bg-gray-100"
                  dangerouslySetInnerHTML={{ __html: docxHtml }}
                />
              </div>
            ) : pdfBlobUrl ? (
              <div className="flex flex-col items-center w-full h-full">
                {/* Zoom controls */}
                <div className="flex items-center gap-2 mb-3 bg-gray-800/80 rounded-lg px-3 py-2">
                  <button
                    onClick={zoomOut}
                    disabled={displayWidth <= MIN_DISPLAY_WIDTH}
                    className="p-1.5 hover:bg-gray-700 rounded transition-colors disabled:opacity-30 disabled:cursor-not-allowed"
                    title="Zoom out"
                  >
                    <ZoomOut size={18} />
                  </button>
                  <button
                    onClick={resetZoom}
                    className="px-2 py-1 text-sm font-medium hover:bg-gray-700 rounded transition-colors min-w-[50px]"
                    title="Reset zoom"
                  >
                    {zoomPercent}%
                  </button>
                  <button
                    onClick={zoomIn}
                    disabled={displayWidth >= MAX_DISPLAY_WIDTH}
                    className="p-1.5 hover:bg-gray-700 rounded transition-colors disabled:opacity-30 disabled:cursor-not-allowed"
                    title="Zoom in"
                  >
                    <ZoomIn size={18} />
                  </button>
                  <button
                    onClick={resetZoom}
                    disabled={displayWidth === DEFAULT_DISPLAY_WIDTH}
                    className="p-1.5 hover:bg-gray-700 rounded transition-colors ml-1 disabled:opacity-30 disabled:cursor-not-allowed"
                    title="Reset to default"
                  >
                    <RotateCcw size={16} />
                  </button>
                  <span className="text-xs text-gray-500 ml-2">
                    Page {pageNumber} of {numPages}
                  </span>
                </div>

                {/* Scrollable PDF area */}
                <div className="flex-1 overflow-auto w-full flex justify-center">
                  <div className="flex items-start gap-3">
                    {/* Main PDF - react-pdf at high scale, CSS scaled down for display */}
                    <Document file={pdfBlobUrl} onLoadSuccess={onDocumentLoadSuccess}>
                      <div
                        className="border border-gray-700 rounded-lg overflow-hidden"
                        style={{ width: displayWidth }}
                      >
                        <div
                          style={{
                            transform: `scale(${cssScale})`,
                            transformOrigin: 'top left',
                          }}
                        >
                          <Page
                            pageNumber={pageNumber}
                            scale={PDF_RENDER_SCALE}
                            renderTextLayer={true}
                            renderAnnotationLayer={true}
                            onRenderSuccess={onPageRenderSuccess}
                          />
                        </div>
                      </div>
                    </Document>

                    {/* Thumbnail sidebar */}
                    <div className="w-24 flex-shrink-0 overflow-y-auto bg-gray-800/50 rounded-lg p-2 max-h-[calc(100vh-380px)]">
                      <Document file={pdfBlobUrl}>
                        {numPages > 0 && Array.from({ length: numPages }, (_, i) => i + 1).map((page) => (
                          <div
                            key={page}
                            onClick={() => setPageNumber(page)}
                            className={`cursor-pointer mb-2 rounded overflow-hidden transition-all ${
                              page === pageNumber
                                ? 'ring-2 ring-amber-500'
                                : 'opacity-50 hover:opacity-100'
                            }`}
                          >
                            <Page
                              pageNumber={page}
                              width={80}
                              renderTextLayer={false}
                              renderAnnotationLayer={false}
                            />
                          </div>
                        ))}
                      </Document>
                    </div>
                  </div>
                </div>
              </div>
            ) : metadata.pdfUrl ? (
              // Fallback: iframe for HTML pages or DOI redirects
              <div className="w-full h-full flex flex-col">
                <div className="flex items-center justify-between mb-2 px-2">
                  <span className="text-sm text-gray-400">Viewing external page</span>
                  <a
                    href={metadata.pdfUrl}
                    target="_blank"
                    rel="noopener noreferrer"
                    className="flex items-center gap-1 text-sm text-amber-400 hover:text-amber-300"
                  >
                    <ExternalLink size={14} />
                    Open in Browser
                  </a>
                </div>
                <iframe
                  src={metadata.pdfUrl}
                  className="w-full flex-1 border border-gray-700 rounded-lg bg-white min-h-[600px]"
                  title={title}
                  sandbox="allow-same-origin allow-scripts allow-popups allow-forms"
                />
              </div>
            ) : null}
          </div>
        )}
      </div>
    </div>
  );
}

export default PaperViewer;
