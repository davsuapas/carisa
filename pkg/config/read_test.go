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
	"os"
	"testing"

	"github.com/stretchr/testify/assert"
)

type TestConfig struct {
	A int `json:",omitempty"`
	B int `json:",omitempty"`
}

func TestConfig_Read(t *testing.T) {
	os.Setenv("CONFIG_VAR", `{
		"a": 1,
		"b": 2
	}`)

	type args struct {
		fichero bool
		ref     string
		confg   TestConfig
	}
	tests := []struct {
		name      string
		args      args
		expectCnf TestConfig
		expectErr bool
	}{
		{
			name: "Read a config file",
			args: args{
				fichero: true,
				ref:     "./rtest/config.json",
				confg: TestConfig{
					A: 2,
				},
			},
			expectCnf: TestConfig{
				A: 1,
			},
			expectErr: false,
		},
		{
			name: "Read a environment variable",
			args: args{
				fichero: false,
				ref:     "CONFIG_VAR",
				confg: TestConfig{
					A: 3,
					B: 4,
				},
			},
			expectCnf: TestConfig{
				A: 1,
				B: 2,
			},
			expectErr: false,
		},
		{
			name: "Envoronment variable not defined",
			args: args{
				fichero: false,
				ref:     "CONFIG_VAR_NOT_DEFINED",
				confg: TestConfig{
					A: 3,
					B: 4,
				},
			},
			expectCnf: TestConfig{
				A: 3,
				B: 4,
			},
			expectErr: false,
		},
		{
			name: "Config file not found",
			args: args{
				fichero: true,
				ref:     "./rtest/co.json",
				confg:   TestConfig{},
			},
			expectCnf: TestConfig{},
			expectErr: true,
		},
	}
	for _, tt := range tests {
		tt := tt
		t.Run(tt.name, func(t *testing.T) {
			t.Parallel()
			if err := Read(tt.args.fichero, tt.args.ref, &tt.args.confg); (err != nil) != tt.expectErr {
				t.Errorf("read() error = %v, expectErr %v", err, tt.expectErr)
			}
			if !tt.expectErr {
				assert.Equal(t, tt.expectCnf, tt.args.confg, "Checking config struct")
			}
		})
	}
}
