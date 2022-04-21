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

package net

import (
	"net"
	"testing"

	"github.com/carisa/pkg/log"
	"github.com/stretchr/testify/assert"
)

func TestTCPHealth_NewTCPHealth_Panic_Bad_Address(t *testing.T) {
	assert.Panics(t, func() { NewTCPHealth(log.TestLogger(), "5:5050") })
}

func TestTCPHealth_Run(t *testing.T) {
	health := NewTCPHealth(log.TestLogger(), "localhost:5050")
	health.Run()
	client, err := net.Dial("tcp", "localhost:5050")
	assert.Nil(t, err)
	client.Close()
	health.Stop()
}
