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
	"go.uber.org/zap"
)

func TestNewLogger(t *testing.T) {
	type args struct {
		config Zap
	}
	tests := []struct {
		name string
		args args
		elog *zap.Logger
	}{
		{
			name: "Development logger",
			args: args{
				config: Zap{
					Development: true,
					Level:       zap.DebugLevel,
					Encoding:    "console",
				},
			},
		},
		{
			name: "Production logger",
			args: args{
				config: Zap{
					Development: false,
					Level:       zap.ErrorLevel,
					Encoding:    "json",
				},
			},
		},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			alog := NewLogger(tt.args.config)
			assert.NotNil(t, alog)
		})
	}
}
