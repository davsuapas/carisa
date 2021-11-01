/*
 *   Copyright (c) 2021 CARISA
 *   All rights reserved.

 *   Licensed under the Apache License, Version 2.0 (the "License");
 *   you may not use this file except in compliance with the License.
 *   You may obtain a copy of the License at

 *   http://www.apache.org/licenses/LICENSE-2.0

 *   Unless required by applicable law or agreed to in writing, software
 *   distributed under the License is distributed on an "AS IS" BASIS,
 *   WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 *   See the License for the specific language governing permissions and
 *   limitations under the License.
 */

package worker

import (
	"encoding/json"
	"flag"

	"github.com/carisa/internal/config"
	"github.com/carisa/internal/servicei"
	configp "github.com/carisa/pkg/config"
	"go.uber.org/zap"
)

var confgFile string

func init() {
	flag.StringVar(&confgFile, "config", "", "the worker config json file")
}

type Config struct {
	// It is the isolation unit where a worker is located.
	Namespace string `json:"namespace,omitempty"`
	config.Common
}

func (c *Config) ToString() string {
	r, _ := json.Marshal(c)
	return string(r)
}

// Factory is the worker controller
type Factory struct {
	config    Config
	discovery servicei.Discovery
	log       *zap.Logger
}

// Build builds worker factory
func FactoryBuild() *Factory {
	file := false
	ref := "WORKER_CONFIG_JSON"

	if len(confgFile) > 0 {
		file = true
		ref = confgFile
	}

	confg := Config{
		Namespace: "",
		Common:    config.Default("worker"),
	}
	if err := configp.Read(file, ref, &confg); err != nil {
		panic(err)
	}

	log := config.NewLogger(confg.Common.Zap)

	log.Info("Loading worker configuration", zap.String("Source", ref), zap.String("Config", confg.ToString()))

	return &Factory{
		config:    confg,
		discovery: servicei.NewConsulDiscovery(log, confg.Discovery),
		log:       log,
	}
}
