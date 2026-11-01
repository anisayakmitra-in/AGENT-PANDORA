package pandora

import (
	"bytes"
	"context"
	"errors"
	"fmt"
	"os/exec"
	"strings"
	"time"
)

// Client provides Go bindings for AGENT-PANDORA.
type Client struct {
	launcherPath string
	workDir      string
	env          map[string]string
	timeout      time.Duration
}

// NewClient initializes a new Pandora client.
func NewClient(opts ClientOptions) (*Client, error) {
	launcher := opts.LauncherPath
	if launcher == "" {
		launcher = "pandora"
	}
	timeout := 120 * time.Second
	if opts.TimeoutSec > 0 {
		timeout = time.Duration(opts.TimeoutSec) * time.Second
	}
	return &Client{
		launcherPath: launcher,
		workDir:      opts.WorkDir,
		env:          opts.Env,
		timeout:      timeout,
	}, nil
}

// Version returns the AGENT-PANDORA Go SDK version.
func (c *Client) Version() string {
	return Version
}

// RunJSON executes a CLI command with `--json` appended and parses the envelope.
func (c *Client) RunJSON(ctx context.Context, args []string) (*Envelope, error) {
	for _, arg := range args {
		if arg == "--json" {
			return nil, errors.New("RunJSON appends '--json' automatically")
		}
	}

	execArgs := append(args, "--json")
	cmdCtx, cancel := context.WithTimeout(ctx, c.timeout)
	defer cancel()

	cmd := exec.CommandContext(cmdCtx, c.launcherPath, execArgs...)
	if c.workDir != "" {
		cmd.Dir = c.workDir
	}

	var stdout, stderr bytes.Buffer
	cmd.Stdout = &stdout
	cmd.Stderr = &stderr

	runErr := cmd.Run()
	output := strings.TrimSpace(stdout.String())

	env, parseErr := ParseEnvelope(output)
	if parseErr != nil {
		if runErr != nil {
			return nil, fmt.Errorf("command execution failed (%v): %s", runErr, stderr.String())
		}
		return nil, fmt.Errorf("failed to parse envelope: %w", parseErr)
	}

	if cmd.ProcessState != nil {
		env.ExitCode = cmd.ProcessState.ExitCode()
	}

	return env, nil
}

// Doctor checks system health via `pandora doctor`.
func (c *Client) Doctor(ctx context.Context) (*DoctorResult, error) {
	env, err := c.RunJSON(ctx, []string{"doctor"})
	if err != nil {
		return nil, err
	}
	return &DoctorResult{
		Version: env.Version,
		Healthy: env.Healthy,
	}, nil
}

// VerifyWorkspace runs workspace verification.
func (c *Client) VerifyWorkspace(ctx context.Context) (*VerificationResult, error) {
	env, err := c.RunJSON(ctx, []string{"verify"})
	if err != nil {
		return nil, err
	}
	return &VerificationResult{
		Version: env.Version,
		Success: env.Healthy || env.ExitCode == 0,
	}, nil
}
