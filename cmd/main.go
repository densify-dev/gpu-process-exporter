// SPDX-License-Identifier: Apache-2.0

package main

import (
	"context"
	"log"
	"os/signal"
	"syscall"

	"github.com/densify-dev/gpu-process-exporter/pkg/config"
	"github.com/densify-dev/gpu-process-exporter/pkg/kube"
	"github.com/densify-dev/gpu-process-exporter/pkg/nvml"
	"github.com/densify-dev/gpu-process-exporter/pkg/prometheus"
)

func main() {
	if err := run(); err != nil {
		log.Fatalf("run exporter: %v", err)
	}
}

func run() error {
	ctx, stop := signal.NotifyContext(context.Background(), syscall.SIGINT, syscall.SIGTERM)
	defer stop()

	cfg, err := config.New()
	if err != nil {
		return err
	}
	gmp := prometheus.NewGpuMetricsProvider()
	store, err := kube.NewStore(ctx, cfg, gmp)
	if err != nil {
		return err
	}

	metricsCollector := nvml.NewMetricsCollector(store, cfg, gmp)
	errCh := make(chan error, 2)
	go func() {
		errCh <- gmp.ListenAndServe(ctx, cfg)
	}()
	go func() {
		errCh <- metricsCollector.Run(ctx)
	}()

	if err = <-errCh; err != nil {
		stop()
		<-errCh
		return err
	}
	return <-errCh
}
