package pandora

import "encoding/json"

// Version is the active SemVer release version of AGENT-PANDORA.
const Version = "3.0.10"

// DenseDimension is the feature hashing dimension for semantic memory.
const DenseDimension = 128

// Envelope represents a standard Pandora JSON-RPC envelope.
type Envelope struct {
	Version  string                 `json:"version"`
	Command  string                 `json:"command,omitempty"`
	Healthy  bool                   `json:"healthy,omitempty"`
	Code     string                 `json:"code,omitempty"`
	Message  string                 `json:"message,omitempty"`
	Details  map[string]interface{} `json:"details,omitempty"`
	ExitCode int                    `json:"-"`
	RawJSON  string                 `json:"-"`
}

// ClientOptions configures the Pandora Go client.
type ClientOptions struct {
	LauncherPath string
	WorkDir      string
	Env          map[string]string
	TimeoutSec   int
}

// DoctorResult is the structured output of `pandora doctor`.
type DoctorResult struct {
	Version string `json:"version"`
	Healthy bool   `json:"healthy"`
}

// VerificationResult is the structured output of `pandora verify` or `workspace.verify`.
type VerificationResult struct {
	Version string `json:"version"`
	Success bool   `json:"success"`
	Cached  bool   `json:"cached,omitempty"`
}

// LspLocation represents an LSP source location.
type LspLocation struct {
	URI  string `json:"uri"`
	Line int    `json:"line"`
	Col  int    `json:"col"`
}

// LspDiagnostic represents a language server compiler diagnostic.
type LspDiagnostic struct {
	File     string `json:"file"`
	Line     int    `json:"line"`
	Col      int    `json:"col"`
	Severity string `json:"severity"`
	Message  string `json:"message"`
}

// SearchResultItem represents a semantic memory hybrid search result.
type SearchResultItem struct {
	DocID string  `json:"doc_id"`
	URI   string  `json:"uri"`
	Score float64 `json:"score"`
}

// ParseEnvelope parses a raw JSON string into a structured Envelope.
func ParseEnvelope(raw string) (*Envelope, error) {
	var env Envelope
	if err := json.Unmarshal([]byte(raw), &env); err != nil {
		return nil, err
	}
	env.RawJSON = raw
	return &env, nil
}
