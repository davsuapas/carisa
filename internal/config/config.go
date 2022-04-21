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

package config

import (
	"github.com/rs/xid"
	"go.uber.org/zap/zapcore"
)

type NodeType string

const (
	Master NodeType = "master"
	Worker          = "worker"
)

// Zap defines the configuration for log framework
type Zap struct {
	// Development mode. Common value: false
	Development bool `json:",omitempty"`
	// Level. See zapcore.Level.
	Level zapcore.Level `json:",omitempty"`
	// Encoding type. Common value: Depending of Development flag
	// The values can be: json -> json format, console -> console format
	Encoding string `json:",omitempty"`
}

// Server defines the server configuration
type Server struct {
	// ID identifies the server
	ID string
	// Address is the address of the server
	Address string `json:",omitempty"`
	// Port is the port of server
	Port int `json:",omitempty"`
	// NodeType can be or 'master' or 'worker'
	NodeType NodeType `json:"-"`
}

// Ckeck checks the server heatlh
type Health struct {
	// Interval specifies the frequency in seconds at which to run this check
	Interval int `json:",omitempty"`
	// Timeout specifies a timeout in seconds for outgoing connections
	// in seconds
	Timeout int `json:",omitempty"`
	// FailuresBeforeCritical specifies the number of consecutive unsuccessful
	// results required before check status transitions to critical.
	FailuresBeforeCritical int `json:",omitempty"`
	// DeregisterCriticalServiceAfter specifies that checks associated
	// with a service should deregister after this time in minutes.
	DeregisterCriticalServiceAfter int `json:",omitempty"`
	// Port is the port for checking
	Port int `json:",omitempty"`
}

// Discovery defines the discovery server
type Discovery struct {
	// Server is the server address
	Server string `json:",omitempty"`
	// Ckeck checks the server heatlh
	Health Health `json:",omitempty"`
}

// Common defines the common config
type Common struct {
	// Zap defines the configuration for log framework
	Zap `json:"log,omitempty"`
	// Discovery defines the discovery server
	Discovery `json:"discovery,omitempty"`
	// Server defines the server configuration
	Server `json:"server,omitempty"`
}

// Default defines the default common config
func Default(typeNode NodeType, port int) Common {
	if port == 0 {
		port = 62422
	}

	return Common{
		Zap: Zap{
			Development: true,
			Level:       zapcore.DebugLevel,
			Encoding:    "console",
		},
		Discovery: DefaultDiscovery(port + 1),
		Server: Server{
			ID:       xid.New().String(),
			Address:  "localhost",
			NodeType: typeNode,
			Port:     port,
		},
	}
}

func DefaultDiscovery(port int) Discovery {
	return Discovery{
		Health: Health{
			Interval:                       10,
			Timeout:                        5,
			FailuresBeforeCritical:         2,
			DeregisterCriticalServiceAfter: 1,
			Port:                           port,
		},
	}
}
