/*
 *   Copyright (c) 2022 CARISA
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

package master

import (
	"encoding/json"

	"github.com/carisa/internal/config"
	"github.com/carisa/internal/net"
	configp "github.com/carisa/pkg/config"
	netp "github.com/carisa/pkg/net"
	"go.uber.org/zap"
)

type Config struct {
	config.Common
}

func (c *Config) ToString() string {
	r, _ := json.Marshal(c)
	return string(r)
}

// Factory is the master controller
type Factory struct {
	config    Config
	discovery net.Discovery
	health    netp.Health
	log       *zap.Logger
}

const MasterPort int = 52422

// Build builds master factory
func FactoryBuild(configFile string) *Factory {
	file := false
	ref := "CARISA_MASTER_CONFIG_JSON"

	if len(configFile) > 0 {
		file = true
		ref = configFile
	}

	cnf := Config{
		Common: config.Default(config.Master, MasterPort),
	}
	if err := configp.Read(file, ref, &cnf); err != nil {
		panic(err)
	}

	log := config.NewLogger(cnf.Common.Zap)

	log.Info("Loading master configuration", zap.String("Source", ref), zap.String("Config", cnf.ToString()))

	return &Factory{
		config:    cnf,
		discovery: net.NewConsulDiscovery(log, cnf.Discovery.Server),
		health:    netp.NewTCPHealth(log, net.HealthAddress(cnf.Server, cnf.Health)),
		log:       log,
	}
}
