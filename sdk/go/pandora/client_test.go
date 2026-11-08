package pandora

import (
	"context"
	"testing"
)

func TestGoSdkVersion(t *testing.T) {
	if Version != "3.0.7" {
		t.Fatalf("expected version 3.0.7, got %s", Version)
	}
}

func TestParseEnvelopeSuccess(t *testing.T) {
	raw := `{"version":"3.0.7","command":"doctor","healthy":true}`
	env, err := ParseEnvelope(raw)
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if env.Version != "3.0.7" {
		t.Errorf("expected version 3.0.7, got %s", env.Version)
	}
	if env.Command != "doctor" {
		t.Errorf("expected command doctor, got %s", env.Command)
	}
	if !env.Healthy {
		t.Errorf("expected healthy true, got false")
	}
}

func TestParseEnvelopeError(t *testing.T) {
	raw := `{"version":"3.0.7","code":"policy_denied","message":"effect denied","details":{"path":"/root"}}`
	env, err := ParseEnvelope(raw)
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if env.Code != "policy_denied" {
		t.Errorf("expected code policy_denied, got %s", env.Code)
	}
	if env.Message != "effect denied" {
		t.Errorf("expected message 'effect denied', got %s", env.Message)
	}
}

func TestClientLifecycle(t *testing.T) {
	client, err := NewClient(ClientOptions{
		LauncherPath: "pandora",
		TimeoutSec:   30,
	})
	if err != nil {
		t.Fatalf("failed to create client: %v", err)
	}
	if client.Version() != "3.0.7" {
		t.Errorf("expected client version 3.0.7, got %s", client.Version())
	}
}
