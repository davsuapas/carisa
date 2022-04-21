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
	"testing"

	"github.com/stretchr/testify/assert"
	"go.uber.org/zap/zapcore"
)

func TestDefault(t *testing.T) {
	type args struct {
		typeNode NodeType
		port     int
		res      Common
	}
	tests := []struct {
		name string
		args args
		res  Common
	}{
		{
			name: "Default with port equal to 0",
			args: args{
				typeNode: Worker,
				port:     0,
			},
			res: Common{
				Zap: Zap{
					Development: true,
					Level:       zapcore.DebugLevel,
					Encoding:    "console",
				},
				Discovery: DefaultDiscovery(62422 + 1),
				Server: Server{
					Port: 62422,
				},
			},
		},
		{
			name: "Default with port configured",
			args: args{
				typeNode: Master,
				port:     3030,
			},
			res: Common{
				Zap: Zap{
					Development: true,
					Level:       zapcore.DebugLevel,
					Encoding:    "console",
				},
				Discovery: DefaultDiscovery(3030 + 1),
				Server: Server{
					Port: 3030,
				},
			},
		},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			d := Default(tt.args.typeNode, tt.args.port)
			assert.Equal(t, tt.res.Zap, d.Zap, "Zap")
			assert.Equal(t, tt.res.Discovery, d.Discovery, "Discovery")
			assert.Equal(t, tt.res.Port, d.Server.Port, "Port")
		})
	}
}
