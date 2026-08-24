// SPDX-License-Identifier: Apache-2.0

package value

import (
	"os"
	"strings"
	"testing"
	"time"
)

func TestParseSupportedTypes(t *testing.T) {
	tm := time.Date(2026, 4, 23, 12, 34, 56, 0, time.UTC)

	tests := []struct {
		name string
		fn   func(t *testing.T)
	}{
		{
			name: "string",
			fn: func(t *testing.T) {
				got, err := Parse[string]("  hello  ")
				if err != nil || got != "hello" {
					t.Fatalf("Parse[string]() = %q, %v; want %q, nil", got, err, "hello")
				}
			},
		},
		{
			name: "bool",
			fn: func(t *testing.T) {
				got, err := Parse[bool](" true ")
				if err != nil || !got {
					t.Fatalf("Parse[bool]() = %v, %v; want true, nil", got, err)
				}
			},
		},
		{
			name: "int",
			fn: func(t *testing.T) {
				got, err := Parse[int](" 42 ")
				if err != nil || got != 42 {
					t.Fatalf("Parse[int]() = %d, %v; want 42, nil", got, err)
				}
			},
		},
		{
			name: "int64 duration-underlying",
			fn: func(t *testing.T) {
				got, err := Parse[time.Duration]("1500ms")
				if err != nil || got != 1500*time.Millisecond {
					t.Fatalf("Parse[time.Duration]() = %v, %v; want 1500ms, nil", got, err)
				}
			},
		},
		{
			name: "uint64",
			fn: func(t *testing.T) {
				got, err := Parse[uint64](" 99 ")
				if err != nil || got != 99 {
					t.Fatalf("Parse[uint64]() = %d, %v; want 99, nil", got, err)
				}
			},
		},
		{
			name: "float64",
			fn: func(t *testing.T) {
				got, err := Parse[float64](" 1.25 ")
				if err != nil || got != 1.25 {
					t.Fatalf("Parse[float64]() = %v, %v; want 1.25, nil", got, err)
				}
			},
		},
		{
			name: "complex128",
			fn: func(t *testing.T) {
				got, err := Parse[complex128]("1+2i")
				if err != nil || got != complex(1, 2) {
					t.Fatalf("Parse[complex128]() = %v, %v; want (1+2i), nil", got, err)
				}
			},
		},
		{
			name: "time",
			fn: func(t *testing.T) {
				got, err := Parse[time.Time](tm.Format(time.RFC3339))
				if err != nil || !got.Equal(tm) {
					t.Fatalf("Parse[time.Time]() = %v, %v; want %v, nil", got, err, tm)
				}
			},
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, tt.fn)
	}
}

func TestParseRejectsEmptyAndInvalidValues(t *testing.T) {
	if _, err := Parse[string]("   "); err == nil || !strings.Contains(err.Error(), "empty") {
		t.Fatalf("Parse[string](empty) error = %v, want empty-string error", err)
	}

	if _, err := Parse[int]("not-an-int"); err == nil {
		t.Fatal("Parse[int](invalid) error = nil, want error")
	}
}

func TestParseOrDefaultAndProviderHelpers(t *testing.T) {
	provider := MapProvider(map[string]string{
		"bool":   "true",
		"number": "7",
		"bad":    "oops",
	})

	if got := ParseOrDefault("bad", 123); got != 123 {
		t.Fatalf("ParseOrDefault() = %d, want 123", got)
	}
	if got, err := ParseValue[bool](provider, "bool"); err != nil || !got {
		t.Fatalf("ParseValue[bool]() = %v, %v; want true, nil", got, err)
	}
	if got, ok := FindParsedValue[int](provider, "number"); !ok || got != 7 {
		t.Fatalf("FindParsedValue[int]() = %d, %v; want 7, true", got, ok)
	}
	if _, ok := FindParsedValue[int](provider, "bad"); ok {
		t.Fatal("FindParsedValue[int](bad) ok = true, want false")
	}
	if got := ParseValueOrDefault(provider, "missing", 55); got != 55 {
		t.Fatalf("ParseValueOrDefault(missing) = %d, want 55", got)
	}
}

func TestEnvProviderAndMapProvider(t *testing.T) {
	t.Setenv("VALUE_TEST_ENV", "env-value")

	if got := EnvProvider().GetValue("VALUE_TEST_ENV"); got != "env-value" {
		t.Fatalf("EnvProvider().GetValue() = %q, want %q", got, "env-value")
	}

	if got := MapProvider(map[string]string{"k": "v"}).GetValue("k"); got != "v" {
		t.Fatalf("MapProvider().GetValue() = %q, want %q", got, "v")
	}

	var mp *mapProvider
	if got := mp.GetValue("missing"); got != "" {
		t.Fatalf("nil mapProvider GetValue() = %q, want empty string", got)
	}
}

func TestEnvProviderReflectsProcessEnvironment(t *testing.T) {
	const key = "VALUE_TEST_PROCESS_ENV"
	if err := os.Setenv(key, "setenv-value"); err != nil {
		t.Fatalf("os.Setenv() error = %v", err)
	}
	defer func() {
		_ = os.Unsetenv(key)
	}()

	if got := EnvProvider().GetValue(key); got != "setenv-value" {
		t.Fatalf("EnvProvider().GetValue() = %q, want %q", got, "setenv-value")
	}
}
