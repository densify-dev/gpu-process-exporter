// SPDX-License-Identifier: Apache-2.0

// Package value provides utilities for:
//  1. Parsing string values (either from environment variables or maps, typically k8s annotations) using generics
//  2. Hashing []string (typically Prometheus labels) to create a fingerprint
package value

import (
	"fmt"
	"os"
	"strconv"
	"strings"
	"time"
)

const (
	Empty = ""
)

type Value interface {
	// time.Duration is int64, so should not appear here separately
	~string | ~bool | ~int | ~int64 | ~uint64 | ~float64 | ~complex128 | time.Time
}

func Parse[T Value](strValue string) (t T, err error) {
	trimmed := strings.TrimSpace(strValue)
	if trimmed == Empty {
		err = fmt.Errorf("cannot parse an empty (trimmed) string")
		return
	}
	switch v := any(t).(type) {
	case string:
		t = any(trimmed).(T)
	case bool:
		var b bool
		if b, err = strconv.ParseBool(trimmed); err == nil {
			t = any(b).(T)
		}
	case int:
		var i int
		if i, err = strconv.Atoi(trimmed); err == nil {
			t = any(i).(T)
		}
	case int64:
		var i int64
		if i, err = strconv.ParseInt(trimmed, 10, 64); err == nil {
			t = any(i).(T)
		}
	case uint64:
		var i uint64
		if i, err = strconv.ParseUint(trimmed, 10, 64); err == nil {
			t = any(i).(T)
		}
	case float64:
		var f float64
		if f, err = strconv.ParseFloat(trimmed, 64); err == nil {
			t = any(f).(T)
		}
	case complex128:
		var c complex128
		if c, err = strconv.ParseComplex(trimmed, 128); err == nil {
			t = any(c).(T)
		}
	case time.Time:
		var tm time.Time
		if tm, err = time.Parse(time.RFC3339, trimmed); err == nil {
			t = any(tm).(T)
		}
	case time.Duration:
		var d time.Duration
		if d, err = time.ParseDuration(trimmed); err == nil {
			t = any(d).(T)
		}
	default:
		err = fmt.Errorf("cannot parse string to type %T", v)
	}
	return
}

func ParseOrDefault[T Value](strValue string, defValue T) (t T) {
	var err error
	if t, err = Parse[T](strValue); err != nil {
		t = defValue
	}
	return
}

type Provider interface {
	GetValue(name string) string
}

func ParseValue[T Value](p Provider, name string) (t T, err error) {
	return Parse[T](p.GetValue(name))
}

func FindParsedValue[T Value](p Provider, name string) (t T, ok bool) {
	var err error
	if t, err = ParseValue[T](p, name); err == nil {
		ok = true
	}
	return
}

func ParseValueOrDefault[T Value](p Provider, name string, defValue T) (t T) {
	return ParseOrDefault[T](p.GetValue(name), defValue)
}

type envProvider struct{}

var ep *envProvider

func EnvProvider() Provider {
	return ep
}

func (*envProvider) GetValue(name string) string {
	return os.Getenv(name)
}

type mapProvider struct {
	m map[string]string
}

func MapProvider(m map[string]string) Provider {
	return &mapProvider{m: m}
}

func (mp *mapProvider) GetValue(name string) (s string) {
	if mp != nil {
		s = mp.m[name]
	}
	return
}
